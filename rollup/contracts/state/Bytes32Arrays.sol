//SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

struct Bytes32Array {
	bytes32[] inner;
}

library Bytes32Arrays {
	function get(Bytes32Array memory arr, uint256 index) internal pure returns (bytes32) {
		return arr.inner[index];
	}

	function set(Bytes32Array memory arr, uint256 index, bytes32 val) internal pure {
		arr.inner[index] = val;
	}

	function length(Bytes32Array memory arr) internal pure returns (uint256) {
		return arr.inner.length;
	}

	function push(Bytes32Array memory arr, bytes32 val) internal pure {
		bytes32[] memory newInner = new bytes32[](arr.inner.length + 1);
		for (uint256 i = 0; i < arr.inner.length; i++) {
			newInner[i] = arr.inner[i];
		}
		newInner[arr.inner.length] = val;
		arr.inner = newInner;
	}

	function pop(Bytes32Array memory arr) internal pure returns (bytes32 popped) {
		popped = arr.inner[arr.inner.length - 1];
		bytes32[] memory newInner = new bytes32[](arr.inner.length - 1);
		for (uint256 i = 0; i < newInner.length; i++) {
			newInner[i] = arr.inner[i];
		}
		arr.inner = newInner;
	}
}
