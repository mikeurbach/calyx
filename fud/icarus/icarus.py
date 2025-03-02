import re
import simplejson as sjson
from pathlib import Path

from fud.stages import Stage, SourceType, Source
from fud.utils import shell, TmpDir, log
from fud.stages.verilator.json_to_dat import convert2dat, convert2json
from fud.stages import futil
import fud.errors as errors


class IcarusBaseStage(Stage):
    """
    Stage to run Verilog programs with Icarus Verilog
    """

    name = "icarus-verilog"

    def __init__(self, is_vcd, desc):
        super().__init__(
            src_state="icarus-verilog",
            target_state="vcd" if is_vcd else "dat",
            input_type=SourceType.Path,
            output_type=SourceType.Stream,
            description=desc,
        )
        self.is_vcd = is_vcd
        self.object_name = "main.vvp"

    @staticmethod
    def defaults():
        parent = Path(__file__).parent.resolve()
        test_bench = parent / "./tb.sv"
        return {
            "exec": "iverilog",
            "testbench": str(test_bench.resolve()),
            "round_float_to_fixed": True,
        }

    def _define_steps(self, input_data, builder, config):
        testbench = config["stages", self.name, "testbench"]
        data_path = config.get(("stages", "verilog", "data"))
        cmd = config["stages", self.name, "exec"]

        # Step 1: Make a new temporary directory
        @builder.step()
        def mktmp() -> SourceType.Directory:
            """
            Make temporary directory to store Verilator build files.
            """
            return TmpDir()

        # Step 2a: check if we need verilog.data to be passes
        @builder.step()
        def check_verilog_for_mem_read(verilog_src: SourceType.String):
            """
            Read input verilog to see if `icarus-verilog.data` needs to be set.
            """
            if "readmemh" in verilog_src:
                raise errors.MissingDynamicConfiguration("verilog.data")

        # Step 2: Transform data from JSON to Dat.
        @builder.step()
        def json_to_dat(tmp_dir: SourceType.Directory, json_path: SourceType.Stream):
            """
            Converts a `json` data format into a series of `.dat` files.
            """
            round_float_to_fixed = config["stages", self.name, "round_float_to_fixed"]
            convert2dat(
                tmp_dir.name,
                sjson.load(json_path, use_decimal=True),
                "dat",
                round_float_to_fixed,
            )

        # Step 3: compile with verilator
        cmd = " ".join(
            [
                cmd,
                "-g2012",
                "-o",
                "{exec_path}",
                testbench,
                "{input_path}",
            ]
        )

        @builder.step(description=cmd)
        def compile_with_iverilog(
            input_path: SourceType.Path, tmpdir: SourceType.Directory
        ) -> SourceType.Stream:
            return shell(
                cmd.format(
                    input_path=str(input_path),
                    exec_path=f"{tmpdir.name}/{self.object_name}",
                ),
                stdout_as_debug=True,
            )

        # Step 4: simulate
        @builder.step()
        def simulate(tmpdir: SourceType.Directory) -> SourceType.Stream:
            """
            Simulates compiled icarus verilog program.
            """
            cycle_limit = config["stages", "verilog", "cycle_limit"]
            return shell(
                [
                    f"{tmpdir.name}/{self.object_name}",
                    f"+DATA={tmpdir.name}",
                    f"+CYCLE_LIMIT={str(cycle_limit)}",
                    f"+OUT={tmpdir.name}/output.vcd",
                    f"+NOTRACE={0 if self.is_vcd else 1}",
                ]
            )

        # Step 5(self.vcd == True): extract
        @builder.step()
        def output_vcd(tmpdir: SourceType.Directory) -> SourceType.Stream:
            """
            Return the generated `output.vcd`.
            """
            # return stream instead of path because tmpdir gets deleted
            # before the next stage runs
            return (Path(tmpdir.name) / "output.vcd").open("rb")

        # Step 5(self.vc == False): extract cycles + data
        @builder.step()
        def output_json(
            simulated_output: SourceType.String, tmpdir: SourceType.Directory
        ) -> SourceType.String:
            """
            Convert .dat files back into a json file
            """
            r = re.search(r"Simulated\s+((-)?\d+) cycles", simulated_output)
            cycle_count = int(r.group(1)) if r is not None else 0
            if cycle_count < 0:
                log.warn("Cycle count is less than 0")
            data = {
                "cycles": cycle_count,
                "memories": convert2json(tmpdir.name, "out"),
            }
            return sjson.dumps(data, indent=2, sort_keys=True, use_decimal=True)

        @builder.step()
        def cleanup(tmpdir: SourceType.Directory):
            """
            Cleanup build files
            """
            tmpdir.remove()

        # Schedule
        tmpdir = mktmp()
        # if we need to, convert dynamically sourced json to dat
        if data_path is None:
            check_verilog_for_mem_read(input_data)
        else:
            json_to_dat(tmpdir, Source(Path(data_path), SourceType.Path))
        compile_with_iverilog(input_data, tmpdir)
        stdout = simulate(tmpdir)
        result = None
        if self.is_vcd:
            result = output_vcd(tmpdir)
        else:
            result = output_json(stdout, tmpdir)
        cleanup(tmpdir)
        return result


class FutilToIcarus(futil.FutilStage):
    """
    Stage to transform Calyx into icarus-verilog simulatable Verilog
    """

    # No name since FutilStage already defines names

    def __init__(self):
        super().__init__(
            "icarus-verilog",
            "-b verilog --disable-init --disable-verify",
            "Compile Calyx to Verilog instrumented for simulation",
        )


class IcarusToVCDStage(IcarusBaseStage):
    """
    Stage to generate VCD files by simulating through Icarus
    """

    def __init__(self):
        super().__init__(True, "Runs Verilog programs with Icarus and generates VCD")


class IcarusToJsonStage(IcarusBaseStage):
    """
    Stage to generate VCD files by simulating through Icarus
    """

    def __init__(self):
        super().__init__(
            False,
            "Runs Verilog programs with Icarus and generates JSON memory file",
        )


# Export the defined stages to fud
__STAGES__ = [FutilToIcarus, IcarusToVCDStage, IcarusToJsonStage]
