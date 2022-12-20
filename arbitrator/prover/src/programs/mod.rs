// Copyright 2022, Offchain Labs, Inc.
// For license information, see https://github.com/nitro/blob/master/LICENSE

use crate::{
    binary::{ExportKind, WasmBinary},
    value::{self, FunctionType as ArbFunctionType, Value},
};

use arbutil::Color;
use eyre::{bail, Report, Result};
use std::{fmt::Debug, marker::PhantomData};
use wasmer::{
    wasmparser::Operator, ExportIndex, FunctionMiddleware, GlobalInit, GlobalType, Instance,
    MiddlewareError, ModuleMiddleware, Mutability, Store, Value as WasmerValue,
};
use wasmer_types::{
    FunctionIndex, GlobalIndex, LocalFunctionIndex, ModuleInfo, Pages, SignatureIndex, Type,
};

pub mod config;
pub mod heap;
pub mod meter;
pub mod start;

pub trait ModuleMod {
    fn add_global(&mut self, name: &str, ty: Type, init: GlobalInit) -> Result<GlobalIndex>;
    fn get_signature(&self, sig: SignatureIndex) -> Result<ArbFunctionType>;
    fn get_function(&self, func: FunctionIndex) -> Result<ArbFunctionType>;
    fn move_start_function(&mut self, name: &str) -> Result<()>;
    fn limit_heap(&mut self, limit: Pages) -> Result<()>;
}

pub trait Middleware<M: ModuleMod> {
    type FM<'a>: FuncMiddleware<'a> + Debug;

    fn update_module(&self, module: &mut M) -> Result<()>; // not mutable due to wasmer
    fn instrument<'a>(&self, func_index: LocalFunctionIndex) -> Result<Self::FM<'a>>;
    fn name(&self) -> &'static str;
}

pub trait FuncMiddleware<'a> {
    /// Processes the given operator.
    fn feed<O>(&mut self, op: Operator<'a>, out: &mut O) -> Result<()>
    where
        O: Extend<Operator<'a>>;

    /// The name of the middleware
    fn name(&self) -> &'static str;
}

#[derive(Debug)]
pub struct DefaultFuncMiddleware;

impl<'a> FuncMiddleware<'a> for DefaultFuncMiddleware {
    fn feed<O>(&mut self, op: Operator<'a>, out: &mut O) -> Result<()>
    where
        O: Extend<Operator<'a>>,
    {
        out.extend(vec![op]);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "default middleware"
    }
}

/// This wrapper exists to impl wasmer's `ModuleMiddleware` generically.
/// We can't use `T` directly since we don't define `ModuleMiddleware`,
/// and we need `M` to be part of the type.
#[derive(Debug)]
pub struct MiddlewareWrapper<T, M>(pub T, PhantomData<M>)
where
    T: Middleware<M> + Debug + Send + Sync,
    M: ModuleMod;

impl<T, M> MiddlewareWrapper<T, M>
where
    T: Middleware<M> + Debug + Send + Sync,
    M: ModuleMod,
{
    pub fn new(middleware: T) -> Self {
        Self(middleware, PhantomData)
    }
}

impl<T> ModuleMiddleware for MiddlewareWrapper<T, ModuleInfo>
where
    T: Middleware<ModuleInfo> + Debug + Send + Sync + 'static,
{
    fn transform_module_info(&self, module: &mut ModuleInfo) -> Result<(), MiddlewareError> {
        let error = |err| MiddlewareError::new(self.0.name().red(), format!("{:?}", err));
        self.0.update_module(module).map_err(error)
    }

    fn generate_function_middleware<'a>(
        &self,
        local_function_index: LocalFunctionIndex,
    ) -> Box<dyn wasmer::FunctionMiddleware<'a> + 'a> {
        let worker = self.0.instrument(local_function_index).unwrap();
        Box::new(FuncMiddlewareWrapper(worker, PhantomData))
    }
}

/// This wrapper exists to impl wasmer's `FunctionMiddleware` generically.
/// The logic is analogous to that of `ModuleMiddleware`, except this time
/// we need a phantom marker to parameterize by `T`'s reference's lifetime.
#[derive(Debug)]
pub struct FuncMiddlewareWrapper<'a, T: 'a>(T, PhantomData<&'a T>)
where
    T: FuncMiddleware<'a> + Debug;

impl<'a, T> FunctionMiddleware<'a> for FuncMiddlewareWrapper<'a, T>
where
    T: FuncMiddleware<'a> + Debug,
{
    fn feed(
        &mut self,
        op: Operator<'a>,
        out: &mut wasmer::MiddlewareReaderState<'a>,
    ) -> Result<(), MiddlewareError> {
        let name = self.0.name().red();
        let error = |err| MiddlewareError::new(name, format!("{:?}", err));
        self.0.feed(op, out).map_err(error)
    }
}

impl ModuleMod for ModuleInfo {
    fn add_global(&mut self, name: &str, ty: Type, init: GlobalInit) -> Result<GlobalIndex> {
        let global_type = GlobalType::new(ty, Mutability::Var);
        let name = name.to_owned();
        if self.exports.contains_key(&name) {
            bail!("wasm already contains {}", name.red())
        }
        let index = self.globals.push(global_type);
        self.exports.insert(name, ExportIndex::Global(index));
        self.global_initializers.push(init);
        Ok(index)
    }

    fn get_signature(&self, sig: SignatureIndex) -> Result<ArbFunctionType> {
        let error = Report::msg(format!("missing signature {}", sig.as_u32().red()));
        let ty = self.signatures.get(sig).cloned().ok_or(error)?;
        let ty = value::parser_func_type(ty);
        ty.try_into()
    }

    fn get_function(&self, func: FunctionIndex) -> Result<ArbFunctionType> {
        let index = func.as_u32();
        match self.functions.get(func) {
            Some(sig) => self.get_signature(*sig),
            None => match self.function_names.get(&func) {
                Some(name) => bail!("missing func {} @ index {}", name.red(), index.red()),
                None => bail!("missing func @ index {}", index.red()),
            },
        }
    }

    fn move_start_function(&mut self, name: &str) -> Result<()> {
        if let Some(prior) = self.exports.get(name) {
            bail!("function {} already exists @ index {:?}", name.red(), prior)
        }

        if let Some(start) = self.start_function.take() {
            let export = ExportIndex::Function(start);
            self.exports.insert(name.to_owned(), export);
            self.function_names.insert(start, name.to_owned());
        }
        Ok(())
    }

    fn limit_heap(&mut self, limit: Pages) -> Result<()> {
        if self.memories.len() > 1 {
            bail!("multi-memory extension not supported");
        }
        for (_, memory) in &mut self.memories {
            let bound = memory.maximum.unwrap_or(limit);
            let bound = bound.min(limit);
            memory.maximum = Some(bound);

            if memory.minimum > bound {
                let minimum = memory.minimum.0.red();
                let limit = bound.0.red();
                bail!("module memory minimum {} exceeds limit {}", minimum, limit);
            }
        }
        Ok(())
    }
}

impl<'a> ModuleMod for WasmBinary<'a> {
    fn add_global(&mut self, name: &str, _ty: Type, init: GlobalInit) -> Result<GlobalIndex> {
        let global = match init {
            GlobalInit::I32Const(x) => Value::I32(x as u32),
            GlobalInit::I64Const(x) => Value::I64(x as u64),
            GlobalInit::F32Const(x) => Value::F32(x),
            GlobalInit::F64Const(x) => Value::F64(x),
            ty => bail!("cannot add global of type {:?}", ty),
        };
        if self.exports.contains_key(name) {
            bail!("wasm already contains {}", name.red())
        }
        let name = name.to_owned();
        let index = self.globals.len() as u32;
        self.exports.insert(name, (index, ExportKind::Global));
        self.globals.push(global);
        Ok(GlobalIndex::from_u32(index))
    }

    fn get_signature(&self, sig: SignatureIndex) -> Result<ArbFunctionType> {
        let index = sig.as_u32() as usize;
        let error = Report::msg(format!("missing signature {}", index.red()));
        self.types.get(index).cloned().ok_or(error)
    }

    fn get_function(&self, func: FunctionIndex) -> Result<ArbFunctionType> {
        let mut index = func.as_u32() as usize;

        let sig = if index < self.imports.len() {
            self.imports.get(index).map(|x| &x.offset)
        } else {
            index -= self.imports.len();
            self.functions.get(index)
        };

        let func = func.as_u32();
        match sig {
            Some(sig) => self.get_signature(SignatureIndex::from_u32(*sig)),
            None => match self.names.functions.get(&func) {
                Some(name) => bail!("missing func {} @ index {}", name.red(), func.red()),
                None => bail!("missing func @ index {}", func.red()),
            },
        }
    }

    fn move_start_function(&mut self, name: &str) -> Result<()> {
        if let Some(prior) = self.exports.get(name) {
            bail!("function {} already exists @ index {:?}", name.red(), prior)
        }

        if let Some(start) = self.start.take() {
            let name = name.to_owned();
            self.exports.insert(name.clone(), (start, ExportKind::Func));
            self.names.functions.insert(start, name);
        }
        Ok(())
    }

    fn limit_heap(&mut self, limit: Pages) -> Result<()> {
        if self.memories.len() > 1 {
            bail!("multi-memory extension not supported");
        }
        if let Some(memory) = self.memories.first_mut() {
            let bound = memory.maximum.unwrap_or(limit.0.into());
            let bound = bound.min(limit.0.into());
            memory.maximum = Some(bound);

            if memory.initial > bound {
                let minimum = memory.initial.red();
                let limit = bound.red();
                bail!("module memory minimum {} exceeds limit {}", minimum, limit);
            }
        }
        Ok(())
    }
}

pub trait GlobalMod {
    fn get_global<T>(&self, store: &mut Store, name: &str) -> T
    where
        T: TryFrom<WasmerValue>,
        T::Error: Debug;

    fn set_global<T>(&mut self, store: &mut Store, name: &str, value: T)
    where
        T: Into<WasmerValue>;
}

impl GlobalMod for Instance {
    fn get_global<T>(&self, store: &mut Store, name: &str) -> T
    where
        T: TryFrom<WasmerValue>,
        T::Error: Debug,
    {
        let error = format!("global {} does not exist", name.red());
        let global = self.exports.get_global(name).expect(&error);
        let ty = global.get(store);
        let error = format!("wrong type: {:?}", ty);
        ty.try_into().expect(&error)
    }

    fn set_global<T>(&mut self, store: &mut Store, name: &str, value: T)
    where
        T: Into<WasmerValue>,
    {
        let error = format!("global {} does not exist", name.red());
        let global = self.exports.get_global(name).expect(&error);
        global.set(store, value.into()).unwrap();
    }
}