import importlib
import re
import runpy
import sys
from importlib.metadata import version as package_version

from vrc_live_caption.cli import app

_ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")


def _strip_ansi(text: str) -> str:
    return _ANSI_ESCAPE_RE.sub("", text)


class TestCliRootHelp:
    def test_when_help_flag_is_used__then_it_shows_description_and_version_option(
        self,
        cli_runner,
    ) -> None:
        result = cli_runner.invoke(app, ["--help"])
        output = _strip_ansi(result.output)

        assert result.exit_code == 0
        assert "translation sidecars" in output
        assert "--version" in output

    def test_when_short_help_flag_is_used__then_it_matches_long_help(
        self,
        cli_runner,
    ) -> None:
        long_help = cli_runner.invoke(app, ["--help"])
        short_help = cli_runner.invoke(app, ["-h"])

        assert long_help.exit_code == 0
        assert short_help.exit_code == 0
        assert short_help.output == long_help.output

    def test_when_version_flag_is_used__then_it_prints_package_version(
        self,
        cli_runner,
    ) -> None:
        result = cli_runner.invoke(app, ["--version"])

        assert result.exit_code == 0
        assert result.output.strip() == package_version("vrc-live-caption")


class TestCliMainModuleEntryPoint:
    def test_when_package_is_run_as_main__then_it_invokes_cli_main(
        self,
        monkeypatch,
    ) -> None:
        called = False

        def fake_main() -> None:
            nonlocal called
            called = True

        monkeypatch.setattr("vrc_live_caption.cli.main", fake_main)

        runpy.run_module("vrc_live_caption", run_name="__main__", alter_sys=True)

        assert called is True

    def test_when_main_module_is_imported__then_it_does_not_invoke_cli_main(
        self,
        monkeypatch,
    ) -> None:
        called = False

        def fake_main() -> None:
            nonlocal called
            called = True

        monkeypatch.setattr("vrc_live_caption.cli.main", fake_main)
        sys.modules.pop("vrc_live_caption.__main__", None)

        importlib.import_module("vrc_live_caption.__main__")

        assert called is False
