
(module
    (import "hostio" "wavm_link_module"        (func $link       (param i32)     (result i32)))
    (import "hostio" "wavm_unlink_module"      (func $unlink                                 ))
    (import "hostio" "program_set_ink"         (func $set_ink    (param i32 i64)             ))
    (import "hostio" "program_ink_left"        (func $ink_left   (param i32)     (result i64)))
    (import "hostio" "program_ink_status"      (func $ink_status (param i32)     (result i32)))
    (import "hostio" "program_set_stack"       (func $set_stack  (param i32 i32)             ))
    (import "hostio" "program_stack_left"      (func $stack_left (param i32)     (result i32)))
    (import "hostio" "program_call_main"       (func $user_func  (param i32 i32) (result i32)))
    (import "env" "wavm_halt_and_set_finished" (func $halt                                   ))

    ;; WAVM Module hash
    (data (i32.const 0x000)
        "\a4\73\76\c8\ea\84\f2\58\06\c6\17\83\a4\c1\a0\18\ab\72\5c\8c\03\53\95\db\91\6b\29\ec\3a\b9\43\14") ;; user

    (func $start (local $user i32) (local $internals i32)
        ;; link in user.wat
        i32.const 0
        call $link
        local.set $user

        ;; set gas globals
        local.get $user
        i64.const 65536
        call $set_ink

        ;; get gas
        local.get $user
        call $ink_left
        i64.const 65536
        i64.ne
        (if (then (unreachable)))

        ;; get gas status
        (call $ink_status (local.get $user))
        i32.const 0
        i32.ne
        (if (then (unreachable)))

        ;; set stack global
        local.get $user
        i32.const 1024
        call $set_stack

        ;; get stack
        local.get $user
        call $stack_left
        i32.const 1024
        i32.ne
        (if (then (unreachable)))

        ;; call a successful func in user.wat ($safe)
        local.get $user
        i32.const 1 ;; $safe
        call $user_func
        i32.const 1
        i32.ne
        (if (then (unreachable)))

        ;; recover from an unreachable
        local.get $user
        i32.const 2 ;; $unreachable
        call $user_func
        i32.const 2 ;; indicates failure
        i32.ne
        (if (then (unreachable)))

        ;; push some items to the stack
        i32.const 0xa4b0
        i64.const 0xa4b1
        i32.const 0xa4b2

        ;; recover from an out-of-bounds memory access
        local.get $user
        i32.const 3 ;; $out_of_bounds
        call $user_func
        i32.const 2 ;; indicates failure
        i32.ne
        (if (then (unreachable)))

        ;; drop the items from the stack
        drop
        drop
        drop

        ;; unlink module
        call $unlink
        call $halt
    )
    (start $start)
    (memory 1))
