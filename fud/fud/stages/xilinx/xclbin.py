import logging as log
from pathlib import Path

from fud.stages import Source, SourceType, Stage
from fud.stages.remote_context import RemoteExecution, LocalSandbox
from fud.stages.futil import FutilStage
from fud.utils import shell


class XilinxStage(Stage):
    name = "xclbin"

    def __init__(self):
        super().__init__(
            src_state="futil",
            target_state="xclbin",
            input_type=SourceType.Path,
            output_type=SourceType.Stream,
            description="compiles Calyx programs to Xilinx bitstreams",
        )

        # sub stages to use futil to compile

    def _shell(self, client, cmd, remote_exec):
        """Run a command, either locally or remotely."""
        if remote_exec.use_ssh:
            _, stdout, stderr = client.exec_command(cmd)
            for chunk in iter(lambda: stdout.readline(2048), ""):
                log.debug(chunk.strip())
            log.debug(stderr.read().decode("UTF-8").strip())

        else:
            stdout = shell(cmd, capture_stdout=False)
            log.debug(stdout)

    def _define_steps(self, input_data, builder, config):
        # As a debugging aid, the pass can optionally preserve the
        # (local or remote) sandbox where the Xilinx commands ran.
        save_temps = bool(config["stages", self.name, "save_temps"])

        mode = config["stages", self.name, "mode"]
        device = config["stages", self.name, "device"]

        # remote execution context
        remote_exec = RemoteExecution(builder, self, config)

        # tcl files
        self.gen_xo_tcl = (
            Path(config["global", "futil_directory"])
            / "fud"
            / "bitstream"
            / "gen_xo.tcl"
        )

        package_cmd = (
            "cd {tmpdir} && "
            "mkdir -p xclbin && "
            "/scratch/opt/Xilinx/Vivado/2020.2/bin/vivado "
            "-mode batch "
            "-source gen_xo.tcl "
            "-tclargs xclbin/kernel.xo"
        )

        @builder.step(package_cmd)
        def package_xo(client: SourceType.UnTyped, tmpdir: SourceType.String):
            self._shell(client, package_cmd.format(tmpdir=tmpdir), remote_exec)

        xclbin_cmd = (
            "cd {tmpdir} && "
            "/scratch/opt/Xilinx/Vitis/2020.2/bin/v++ -g "
            "-t {mode} "
            "--platform {device} "
            "--save-temps "
            "--profile.data all:all:all "
            "--profile.exec all:all:all "
            "-lo xclbin/kernel.xclbin "
            "xclbin/kernel.xo"
        )

        @builder.step(xclbin_cmd)
        def compile_xclbin(client: SourceType.UnTyped, tmpdir: SourceType.String):
            """
            Compile XO into xclbin.
            """
            self._shell(
                client,
                xclbin_cmd.format(tmpdir=tmpdir, mode=mode, device=device),
                remote_exec,
            )

        # Schedule
        # External stages called by this stage
        xilinx_stage = FutilStage("xilinx-verilog", "-b xilinx", "")
        xml_futil = FutilStage("xilinx-verilog", "-b xilinx-xml", "")
        kernel_futil = FutilStage(
            "xilinx-verilog", "-b verilog --synthesis -p external", ""
        )

        if remote_exec.use_ssh:
            remote_exec.import_libs()

        # Compile files using external stages
        xilinx = xilinx_stage._define_steps(input_data, builder, config)
        xml = xml_futil._define_steps(input_data, builder, config)
        kernel = kernel_futil._define_steps(input_data, builder, config)

        file_map = {
            xilinx: "toplevel.v",
            kernel: "main.sv",
            xml: "kernel.xml",
            self.gen_xo_tcl: "gen_xo.tcl",
        }
        if remote_exec.use_ssh:
            client, tmpdir = remote_exec.open_and_send(file_map)
        else:
            sandbox = LocalSandbox(builder, save_temps)
            tmpdir = sandbox.create(file_map)
            client = Source(None, SourceType.UnTyped)

        package_xo(client, tmpdir)
        compile_xclbin(client, tmpdir)

        if remote_exec.use_ssh:
            return remote_exec.close_and_get(
                client,
                tmpdir,
                "xclbin/kernel.xclbin",
                keep_tmpdir=save_temps,
            )
        else:
            return sandbox.get_file("xclbin/kernel.xclbin")
