# This directory contains test cases for ndn-lib

# First, a brief description of my understanding of the module structure and some key concepts

* Data is divided into two main categories:

    1. Chunk: Unstructured data blocks
    2. Object: Structured data, in the format of any `json` string
        * There are several built-in Objects: File/Dir, etc.

* ndn-lib mainly contains several interface components:

    1. NamedDataMgr: Responsible for local data storage and retrieval
    2. NdnClient: Responsible for data exchange across NamedDataMgr, devices, and Zones
    3. Can be accessed directly via the `http` protocol
    
* Data organization:

    1. All data (including `Chunk` and any `Object`) can be named with an `ID` (usually some kind of `HASH`) according to specific rules; the naming rules can be defined by the user or use the system default.
    2. Store directly into NamedDataMgr and retrieve by `ID`
    3. The system has a tree structure, and any data can be mounted to any leaf node. The path of this leaf node is the data's `NdnPath`; only one object can be mounted to a node.
    4. `Object` is in `json` format and is also a tree structure. Each child node (sub-object) has a corresponding path (`inner-path`). To retrieve a sub-object individually, you can add the `inner-path` parameter when retrieving the root object.

# Test Case Design

* Based on the above module structure, test cases are designed along the following dimensions:

    1. Data type: Chunk, Object, File
    2. Access interface: NamedDataMgr, NdnClient, http
    3. Retrieval method: ID, NdnPath, inner-path
    4. Device topology: Same `NamedDataMgr`, two `NamedDataMgr`, same `Zone` different devices, cross-`Zone` devices

    *** http: The implementation is basically similar to NdnClient, so copying the code is not meaningful. Considering that there may be SDKs with different implementations in the future (such as Python, JS, etc.), just use the SDKs with different implementations directly ***

    *** Same `Zone` different devices: This function is not implemented yet, so no tests are added for now ***

* Test Environment

    * For cases not involving `zone`, just test locally: `cargo test`
    * For cases involving `zone`, start the standard development environment (including several `zone`s) before running `cargo test`:
        
        1. Local development environment: test.buckyos.io
        2. bob.web3.buckyos.io

# Each test case is implemented in a separate file according to the topology

1. local_signal_ndn_mgr_chunk.rs: Only one local `NamedDataMgr`'s `chunk`
2. local_signal_ndn_mgr_obj.rs: Only one local `NamedDataMgr`'s `Object`
3. local_signal_ndn_mgr_file_api.rs: Only one local `NamedDataMgr`'s `File`
4. local_2_ndn_mgr_chunk.rs: Two local `NamedDataMgr`'s `chunk`
5. local_2_ndn_mgr_obj.rs: Two local `NamedDataMgr`'s `Object`
6. local_2_ndn_mgr_file_api.rs: Two local `NamedDataMgr`'s `File`
7. ndn_2_zone_test_chunk.rs: `chunk` across two `zone`s
8. ndn_2_zone_test_obj.rs: `Object` across two `zone`s
9. ndn_2_zone_file_api.rs: `file` across two `zone`s