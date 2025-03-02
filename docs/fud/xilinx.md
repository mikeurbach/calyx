# Xilinx Toolchain

> Working with vendor EDA toolchains is never a fun experience. Something will
> almost certainly go wrong. If you're at Cornell, you can at least avoid
> installing the tools yourself by using our lab servers, [Gorgonzola][] or [Havarti][].

`fud` can interact with the Xilinx tools ([Vivado][], [Vivado HLS][vhls], and [Vitis][]). There are three main things it can do:

* Synthesize Calyx-generated RTL designs to collect area and resource estimates.
* Compile [Dahlia][] programs via C++ and [Vivado HLS][vhls] for comparison with the Calyx backend.
* Compile Calyx programs for *actual execution* in Xilinx emulation modes or on real FPGA hardware.

You can set `fud` up to use either a local installation of the Xilinx tools or one on a remote server, via SSH.

## Synthesis Only

The simplest way to use the Xilinx tools is to synthesize RTL or HLS designs to collect statistics about them.
This route will not produce actual, runnable executables; see the next section for that.

### Set Up

To set up to **invoke the Xilinx tools over SSH**, first tell `fud` your username and hostname for the server:

    # Vivado
    fud config stages.synth-verilog.ssh_host <hostname>
    fud config stages.synth-verilog.ssh_username <username>
    fud config stages.synth-verilog.remote true

    # Vivado HLS
    fud config stages.vivado-hls.ssh_host <hostname>
    fud config stages.vivado-hls.ssh_username <username>
    fud config stages.vivado-hls.remote true

The server must have `vivado` and `vivado_hls` available on the remote machine's path. (If you need the executable names to be something else, please file an issue.)

To instead **invoke the Xilinx tools locally**, just let `fud` run the `vivado` and `vivado_hls` commands.
You can optionally tell `fud` where these commands exist on your machine:

    fud config stages.synth-verilog.exec <path> # update vivado path
    fud config stages.vivado-hls.exec <path> # update vivado_hls path

Leave the `remote` option unset to use local execution and these `exec` paths (which are ignored for remote execution).

### Run

To run the entire toolchain and extract statistics from RTL synthesis, use the `resource-estimate` target state.
For example:

    fud e --to resource-estimate examples/futil/dot-product.futil

To instead obtain the raw synthesis results, use `synth-files`.

To run the analogous toolchain for Dahlia programs via HLS, use the `hls-estimate` target state:

    fud e --to hls-estimate examples/dahlia/dot-product.fuse

There is also an `hls-files` state for the raw results of Vivado HLS.

## Emulation and Execution

`fud` can also compile Calyx programs for actual execution, either in the Xilinx toolchain's emulation modes or for running on a physical FPGA.
This route involves generating an [AXI][] interface wrapper for the Calyx program and invoking it using [XRT][]'s OpenCL interface.

### Set Up

As above, you can invoke the Xilinx toolchain locally or remotely, via SSH.
To set up SSH execution, you can edit your `config.toml` to add settings like this:

    [stages.xclbin]
    ssh_host = "havarti"
    ssh_username = "als485"
    remote = true

To use local execution, just leave off the `remote = true` line.

You can also set the Xilinx mode and target device:

    [stages.xclbin]
    mode = "hw_emu"
    device = "xilinx_u50_gen3x16_xdma_201920_3"

The options for `mode` are `hw_emu` (simulation) and `hw` (on-FPGA execution).
The device string above is for the [Alveo U50][u50] card, which we have at Cornell. The installed Xilinx card would typically be found under the directory `/opt/xilinx/platforms`, where one would be able to find a device name of interest.

To use hardware emulation, you will also need to configure the `wdb` stage.
It has similar `ssh_host`, `ssh_username`, and `remote` options to the `xclbin` stage.
You will also need to configure the stage to point to your installations of [Vitis][] and [XRT][], like this:

    [stages.wdb]
    xilinx_location: /scratch/opt/Xilinx/Vitis/2020.2
    xrt_location: /opt/xilinx/xrt

### Compile

The first step in the Xilinx toolchain is to generate [an `xclbin` executable file][xclbin].
Here's an example of going all the way from a Calyx program to that:

    fud e examples/futil/dot-product.futil -o foo.xclbin

On our machines, compiling even a simple example like the above for simulation takes about 5 minutes, end to end.
A failed run takes about 2 minutes to produce an error.

By default, the Xilinx tools run in a temporary directory that is deleted when `fud` finishes.
To instead keep the sandbox directory, use `-s xclbin.save_temps true`.
You can then find the results in a directory named `fud-out-N` for some number `N`.

### Emulate

You can also execute compiled designs through Xilinx hardware emulation.
Use the `wdb` state as your `fud` target:

    fud e -vv foo.xclbin -s wdb.save_temps true -o out.wdb

This stage produces a Vivado [waveform database (WDB) file][wdb]
Through the magic of `fud`, you can also go all the way from a Calyx program to a `wdb` file in the same way.
There is also a `wdb.save_temps` option, as with the `xclbin` stage.

You also need to provide a host C++ program via the `wdb.host` parameter, but I don't know much about that yet, so documentation about that will have to wait.
Similarly, I don't yet know what you're supposed to *do* with a WDB file; maybe we should figure out how to produce a VCD instead.

### How it Works

The first step is to generate input files.
We need to generate:

* The RTL for the design itself, using the compile command-line flags `-b verilog --synthesis -p external`. We name this file `main.sv`.
* A Verilog interface wrapper, using `XilinxInterfaceBackend`, via `-b xilinx`. We call this `toplevel.v`.
* An XML document describing the interface, using `XilinxXmlBackend`, via `-b xilinx-xml`. This file gets named `kernel.xml`.

The `fud` driver gathers these files together in a sandbox directory.
The next step is to run the Xilinx tools.

The rest of this section describes how this workflow works under the hood.
If you want to follow along by typing commands manually, you can start by invoking the setup scripts for [Vitis][] and [XRT][]:

    source <Vitis_install_path>/Vitis/2020.1/settings64.sh
    source /opt/xilinx/xrt/setup.sh

On some Ubuntu setups, you may need to update `LIBRARY_PATH`:

    export LIBRARY_PATH=/usr/lib/x86_64-linux-gnu

You can check that everything is working by typing `vitis -version` or `vivado -version`.

#### Background: `.xo` and `.xclbin`

In the Xilinx toolchain, compilation to an executable bitstream (or simulation blob) appears to requires two steps:
taking your Verilog sources and creating an `.xo` file, and then taking that and producing an `.xclbin` “executable” file.
The idea appears to be a kind of metaphor for a standard C compilation workflow in software-land: `.xo` is like a `.o` object file, and `.xclbin` contains actual executable code (bitstream or emulation equivalent), like a software executable binary.
Going from Verilog to `.xo` is like “compilation” and going from `.xo` to `.xclbin` is like “linking.”

However,  this analogy is kind of a lie.
Generating an `.xo` file actually does very little work:
it just packages up the Verilog source code and some auxiliary files.
An `.xo` is literally a zip file with that stuff packed up inside.
All the actual work happens during “linking,” i.e., going from `.xo` to `.xclbin` using the `v++` tool.
This situation is a poignant reminder of how impossible separate compilation is in the EDA world.
A proper analogy would involve separately compiling the Verilog into some kind of low-level representation, and then linking would properly smash together those separately-compiled objects.
Instead, in Xilinx-land, “compilation” is just simple bundling and “linking” does all the compilation in one monolithic step.
It’s kind of cute that the Xilinx toolchain is pretending the world is otherwise, but it’s also kind of sad.

Anyway, the only way to produce a `.xo` file from RTL code appears to be to use Vivado (i.e., the actual `vivado` program).
Nothing from the newer Vitis package currently appears capable of producing `.xo` files from Verilog (although `v++` can produce these files during HLS compilation, presumably by invoking Vivado).

The main components in an `.xo` file, aside from the Verilog source code itself, are two XML files:
`kernel.xml`, a short file describing the argument interfaces to the hardware design,
and `component.xml`, a much longer and more complicated [IP-XACT][] file that also has to do with the interface to the RTL.
We currently generate `kernel.xml` ourselves (with the `xilinx-xml` backend described above) and then use Vivado, via a Tcl script, to generate the IP-XACT file.

In the future, we could consider trying to route around using Vivado by generating the IP-XACT file ourselves, using a tool such as [DUH][].

#### Our Workflow

The first step is to produce an `.xo` file.
We also use [a static Tcl script, `gen_xo.tcl`,][gen_xo] which is a simplified version of [a script from Xilinx's Vitis tutorials][package_kernel].
The gist of this script is that it creates a Vivado project, adds the source files, twiddles some settings, and then uses the [`package_xo` command][package_xo] to read stuff from this project as an "IP directory" and produce an `.xo` file.
The Vivado command line looks roughly like this:

    vivado -mode batch -source gen_xo.tcl -tclargs xclbin/kernel.xo

That output filename after `-tclargs`, unsurprisingly, gets passed to [`gen_xo.tcl`][gen_xo].

Then, we take this `.xo` and turn it into an [`.xclbin`][xclbin].
This step uses the `v++` tool, with a command line that looks like this:

    v++ -g -t hw_emu --platform xilinx_u50_gen3x16_xdma_201920_3 --save-temps --profile.data all:all:all --profile.exec all:all:all -lo xclbin/kernel.xclbin xclbin/kernel.xo

Fortunately, the `v++` tool doesn't need any Tcl to drive it; all the action happens on the command line.

[vivado]: https://www.xilinx.com/products/design-tools/vivado.html
[vhls]: https://www.xilinx.com/products/design-tools/vivado/integration/esl-design.html
[gorgonzola]: https://capra.cs.cornell.edu/private/gorgonzola.html
[havarti]: https://capra.cs.cornell.edu/private/havarti.html
[vitis]: https://www.xilinx.com/products/design-tools/vitis/vitis-platform.html
[dahlia]: https://capra.cs.cornell.edu/dahlia/
[axi]: https://en.wikipedia.org/wiki/Advanced_eXtensible_Interface
[xrt]: https://xilinx.github.io/XRT/
[xclbin]: https://xilinx.github.io/XRT/2021.2/html/formats.html#xclbin
[gen_xo]: https://github.com/cucapra/calyx/blob/master/fud/bitstream/gen_xo.tcl
[u50]: https://www.xilinx.com/products/boards-and-kits/alveo/u50.html
[wdb]: https://support.xilinx.com/s/article/64000?language=en_US
[vitis_tutorial]: https://github.com/Xilinx/Vitis-Tutorials/blob/2021.2/Getting_Started/Vitis/Part2.md
[ip-xact]: https://en.wikipedia.org/wiki/IP-XACT
[duh]: https://github.com/sifive/duh
[package_kernel]: https://github.com/Xilinx/Vitis-Tutorials/blob/2021.1/Hardware_Acceleration/Feature_Tutorials/01-rtl_kernel_workflow/reference-files/scripts/package_kernel.tcl
[package_xo]: https://docs.xilinx.com/r/en-US/ug1393-vitis-application-acceleration/package_xo-Command
