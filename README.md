<picture>
  <source media="(prefers-color-scheme: dark)" srcset="logo_light.svg">
  <source media="(prefers-color-scheme: light)" srcset="logo_dark.svg">
  <img alt="OpenVAF" src="logo_dark.svg">
</picture>

<br>    
<br>
<br>

OpenVAF is a Verilog-A compiler that can compile Verilog-A files for use in circuit simulator.
The major aim of this Project is to provide a high-quality standard compliant compiler for Verilog-A.
Furthermore, the project aims to bring modern compiler construction algorithms/data structures to a field with a lack of such tooling.

Some highlights of OpenVAF include:

* **fast compile** times (usually below 1 second for most compact models)
* high-quality **user interface**
* **easy setup** (no runtime dependencies even for cross compilation)
* **fast simulations** surpassing existing solutions by 30%-60%, often matching handwritten models
* IDE aware design

Detailed documentation, examples and precompiled binaries of all release are **available on our [website](https://openvaf.semimod.de)**. To test the latest development version you can download nightly version of OpenVAF for [linux](https://openva.fra1.digitaloceanspaces.com/openvaf_devel-x86_64-unknown-linux-gnu.tar.gz) or [windows](https://openva.fra1.digitaloceanspaces.com/openvaf_devel-x86_64-pc-windows-msvc.tar.gz).


## Projects

The development of OpenVAF and related tools is tightly coupled and therefore happens in a single repository.
This repository currently contains the following useable projects:

### OpenVAF

OpenVAF is the main project of the repository and all other tools use OpenVAF as a library in some form.
OpenVAF can be build as a standalone CLI program that can compile Verilog-A files to shared objects that comply with the simulator independent OSDI interface.

OpenVAF has been tested with a preliminary NGSPICE prototype and Melange.
It can already support a large array of compact models.
However, due to the larger feature set (compared to VerilogAE) additional testing and verification is still required.
Furthermore, some Verilog-A language features are currently not supported.

### VerilogAE

Allows obtaining model equations (calculates the value of a single Variable) from Verilog-A files.
VerilogAE is primarily useable from python (available on pypi) but can also be compiled as a static/shared library and called by any programming language (that supports the C ABI).

VerilogAE is used in production at SemiMod and various partners.
Its stable and ready for general use.

### Melange

Melange is an experimental circuit simulator that leverage OpenVAF to gain access to compact models.
This allows it to easily support a large array of models without the huge effort associated with traditional simulators.
The focus of Melange is primarily on automation.
That means Melange focuses on providing an ergonomic and extensible API in mainstream programming languages (python and rust currently) instead of a special purpose netlist format.
However, to remain compatible with existing PDKs a subset of the spectre netlist format can be parsed.

Melange is currently in early development and most features are not complete.
Some mockups of planned usage can be found in `melange/examples`.
Some working minimal examples (in rust) can be found in `melange/core/test.rs`.

## Building OpenVAF with docker

The official docker image contains everything required for compiling OpenVAF. To build OpenVAF using the official docker containers, simply run the following commands:

``` shell
git clone https://github.com/gjcoram/OpenVAF.git && cd OpenVAF
# On REHL distros and fedora replace docker with podman
# on all commands below. 
docker pull ghcr.io/pascalkuthe/ferris_ci_build_x86_64-unknown-linux-gnu:latest
# On Linux distros that enable SELinux linux RHEL based distros and fedora use $(pwd):/io:Z
docker run -ti -v $(pwd):/io ghcr.io/pascalkuthe/ferris_ci_build_x86_64-unknown-linux-gnu:latest

# Now you are inside the docker container
cd /io
cargo build --release
# OpenVAF will be build this can take a while
# afterwards the binary is available in target/release/openvaf
# inside the repository 
```

## Build Instructions 

OpenVAF **requires rust/cargo 1.64 or newer** (best installed with [rustup](https://rustup.rs/)). Furthermore, the **LLVM-15** development libraries and **clang-15** are required. Newer version also work but older versions of LLVM/clang are not supported. Note that its imperative that **you clang version matches your LLVM version**. If you want to compile VerilogAE python 3.8+ is also required.


On Debian and Ubuntu the [LLVM Project provided packages](https://apt.llvm.org/) can be used:

``` shell
sudo bash -c "$(wget -O - https://apt.llvm.org/llvm.sh)"
```

On fedora (37+) you can simply install LLVM from the default repositories:

``` shell
sudo dnf install clang llvm-devel
```

On Linux distributions where packages for LLVM-15 are not available (like centos-7) and windows you can download the prebuild LLVM packages that are also used
in our docker images. These binaries are build on centos-7 and will therefore
work on any linux distribution (that is supported by rustc):

* [clang and LLVM for windows](https://openva.fra1.cdn.digitaloceanspaces.com/llvm-15.0.7-x86_64-pc-windows-msvc-FULL.tar.zst)
* [clang and LLVM for linux](https://openva.fra1.cdn.digitaloceanspaces.com/llvm-15.0.7-x86_64-unknown-linux-gnu-FULL.tar.zst)

Simply download and extract these tar archives and set the `LLLVM_CONFIG` (see below) environment variable to point at the downloaded files: `LLVM_CONFIG=<extracted directory>/bin/llvm-config`.

> Note: To decompress this archive you need to use the following command:
> ``` shell
> zstd -d -c --long=31 <path/to/archive.tar.zst> | tar -xf -
> ```
> afterwards the decompressed files can be found in `./LLVM`


Once all dependencies have been installed you can build the entire project by running:

``` shell
cargo build --release
```

To only build OpenVAF (and ignore melange/VerilogAE) you can run:

``` shell
cargo build --release --bin openvaf
```

By default, OpenVAF will link against the static LLVM libraries, to avoid runtime dependencies. This is great for creating portable binaries but sometimes building with shared libraries is preferable. Simply set the `LLVM_LINK_SHARED` environment variable during linking to use the shared system libraries. If multiple LLVM versions are installed (often the case on debian) the `LLVM_CONFIG` environment variable can be used to specify the path of the correct `llvm-config` binary.
An example build invocation using shared libraries on debian is shown below:

``` shell
LLVM_LINK_SHARED=1 LLVM_CONFIG="llvm-config-15" cargo build --release
```

OpenVAF includes many integration and unit tests inside its source code.
For development [cargo-nexttest](https://nexte.st/) is recommended to run these tests as it significantly reduces the test runtime.
However, the built-in cargo test runner (requires no extra installation) can also be used.
To run the testsuite simply call:

``` shell
cargo test # default test runner, requires no additional installation
cargo nextest run # using cargo-nextest, much faster but must be installed first
```

By default, the test suite will skip slow integration tests that compile entire compact models.
These can be enabled by setting the `RUN_SLOW_TESTS` environment variable:

``` shell
RUN_SLOW_TESTS=1 cargo nextest run 
```

During development, you likely don't want to run full release builds as these
can take a while to build. Debug builds are much faster:
``` shell
cargo build # debug build
cargo run --bin opnevaf test.va # create a debug build and run it
cargo clippy # check the sourcecode for errors/warnings without building (even faster)
```

## Acknowledgement

The architectures of the [rust-analyzer](https://github.com/rust-analyzer/rust-analyzer) and [rustc](https://github.com/rust-lang/rust/) have heavily inspired the design of this compiler.

## Copyright

This work is free software and licensed under the GPL-3.0 license.
It contains code that is derived from [rustc](https://github.com/rust-lang/rust/) and [rust-analyzer](https://github.com/rust-analyzer/rust-analyzer). These projects are both licensed under the MIT license. As required a copy of the license and disclaimer can be found in `copyright/LICENSE_MIT`.

Many models int integration tests folder are not licensed under a GPL compatible license. All of those models contain explicit license information. They do not endup in the openvaf binary in any way and therefore do not affect the license of the entire project. Integration tests without explicit model information (either in the model files or in a dedicated LICENSE file) fall under GPLv3.0 like the rest of the repo.
