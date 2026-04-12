import logging
from pathlib import Path

from tests.support.fakes.audio import FakeAudioBackend
from tests.support.fakes.osc import FakeOscTransport
from vrc_live_caption.cli import app
from vrc_live_caption.config import LoggingConfig, LogLevel
from vrc_live_caption.errors import OscError, SecretError
from vrc_live_caption.stt.funasr_local import FunasrLocalReadyEvent
from vrc_live_caption.translation.translategemma_local import (
    TranslateGemmaLocalReadyEvent,
)


def _iflytek_env() -> dict[str, str]:
    return {
        "IFLYTEK_APP_ID": "app-id",
        "IFLYTEK_API_KEY": "api-key",
        "IFLYTEK_API_SECRET": "api-secret",
    }


def _openai_env() -> dict[str, str]:
    return {
        "OPENAI_API_KEY": "test-key",
    }


class TestDoctorCommand:
    def test_when_config_is_missing__then_it_warns_and_continues(
        self,
        monkeypatch,
        tmp_cwd: Path,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        missing_config = tmp_cwd / "missing.toml"
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend",
            lambda: backend,
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(missing_config)],
            env=_openai_env(),
        )

        assert result.exit_code == 0
        assert "[warn] config missing" in result.output
        assert "[ok] stream probe succeeded" in result.output
        assert "[ok] osc target configured: 127.0.0.1:9000" in result.output
        assert "[ok] required STT secrets found" in result.output
        assert "[ok] translation disabled" in result.output

    def test_when_iflytek_secrets_are_missing__then_it_exits_non_zero(
        self,
        monkeypatch,
        tmp_cwd: Path,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory()
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )
        monkeypatch.delenv("IFLYTEK_APP_ID", raising=False)
        monkeypatch.delenv("IFLYTEK_API_KEY", raising=False)
        monkeypatch.delenv("IFLYTEK_API_SECRET", raising=False)

        result = cli_runner.invoke(app, ["doctor", "--config", str(config_path)])

        assert result.exit_code == 1
        assert "IFLYTEK_APP_ID not found" in result.output

    def test_when_device_resolution_fails__then_it_prints_the_devices_hint(
        self,
        monkeypatch,
        tmp_cwd: Path,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(capture_overrides={"device": "Missing Mic"})
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )

        result = cli_runner.invoke(app, ["doctor", "--config", str(config_path)])

        assert result.exit_code == 1
        assert "[error] input device resolution failed" in result.output
        assert "Hint: run `vrc-live-caption devices`" in result.output

    def test_when_probe_fails__then_it_exits_non_zero(
        self,
        monkeypatch,
        tmp_cwd: Path,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend(probe_error="probe failed")
        osc_transport = FakeOscTransport()
        config_path = config_file_factory()
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(config_path)],
            env=_iflytek_env(),
        )

        assert result.exit_code == 1
        assert "[error] stream probe failed: probe failed" in result.output

    def test_when_log_level_flags_are_passed__then_they_override_config(
        self,
        monkeypatch,
        tmp_cwd: Path,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory()
        captured: dict[str, LoggingConfig] = {}

        def fake_configure_logging(config: LoggingConfig):
            captured["config"] = config
            return logging.getLogger("test.cli.logging")

        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.configure_logging",
            fake_configure_logging,
        )

        result = cli_runner.invoke(
            app,
            [
                "doctor",
                "--config",
                str(config_path),
                "--file-log-level",
                "debug",
                "--console-log-level",
                "error",
            ],
            env=_iflytek_env(),
        )

        assert result.exit_code == 0
        assert captured["config"].console_level == LogLevel.ERROR
        assert captured["config"].file_level == LogLevel.DEBUG

    def test_when_openai_backend_has_a_key__then_it_accepts_the_runtime(
        self,
        monkeypatch,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(stt_overrides={"provider": "openai_realtime"})
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(config_path)],
            env={"OPENAI_API_KEY": "test-key"},
        )

        assert result.exit_code == 0
        assert "STT backend: openai_realtime" in result.output

    def test_when_local_funasr_backend_is_selected__then_it_skips_secret_validation(
        self,
        monkeypatch,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(stt_overrides={"provider": "funasr_local"})
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )

        async def fake_probe(**kwargs):
            return FunasrLocalReadyEvent(
                message="ready",
                resolved_device="cuda:0",
                device_policy="auto",
            )

        monkeypatch.setattr(
            "vrc_live_caption.cli.probe_funasr_local_service", fake_probe
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.validate_stt_secrets",
            lambda **kwargs: (_ for _ in ()).throw(
                AssertionError("should not be called")
            ),
        )

        result = cli_runner.invoke(app, ["doctor", "--config", str(config_path)])

        assert result.exit_code == 0
        assert "STT backend: funasr_local (127.0.0.1:10095)" in result.output
        assert (
            "[ok] local STT sidecar reachable: "
            "endpoint=ws://127.0.0.1:10095, device=cuda:0, policy=auto" in result.output
        )

    def test_when_osc_configuration_fails__then_it_exits_non_zero(
        self,
        monkeypatch,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        config_path = config_file_factory()
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: (_ for _ in ()).throw(OscError("bad osc")),
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(config_path)],
            env=_iflytek_env(),
        )

        assert result.exit_code == 1
        assert "[error] osc target configuration failed: bad osc" in result.output

    def test_when_translation_runtime_is_valid__then_it_prints_the_translation_summary(
        self,
        monkeypatch,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(
            translation_overrides={
                "enabled": True,
                "provider": "deepl",
                "target_language": "en",
                "output_mode": "source_target",
            }
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.validate_translation_runtime",
            lambda **kwargs: None,
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(config_path)],
            env=_iflytek_env(),
        )

        assert result.exit_code == 0
        assert (
            "Translation: deepl -> en (mode=source_target, strategy=final_only)"
            in result.output
        )
        assert "[ok] translation runtime validated" in result.output

    def test_when_local_translategemma_translation_is_enabled__then_it_probes_sidecar(
        self,
        monkeypatch,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(
            translation_overrides={
                "enabled": True,
                "provider": "translategemma_local",
                "source_language": "zh",
                "target_language": "en",
                "output_mode": "source_target",
            }
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.probe_translategemma_local_service",
            lambda **kwargs: TranslateGemmaLocalReadyEvent(
                message="ready",
                model="google/translategemma-4b-it",
                resolved_device="cuda:0",
                device_policy="auto",
                resolved_dtype="bfloat16",
            ),
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.validate_translation_runtime",
            lambda **kwargs: (_ for _ in ()).throw(
                AssertionError("should not be called")
            ),
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(config_path)],
            env=_iflytek_env(),
        )

        assert result.exit_code == 0
        assert (
            "[ok] local translation sidecar reachable: "
            "endpoint=ws://127.0.0.1:10096, model=google/translategemma-4b-it, device=cuda:0, policy=auto, dtype=bfloat16"
            in result.output
        )
        assert (
            "Translation: translategemma_local (127.0.0.1:10096) -> en" in result.output
        )

    def test_when_local_translategemma_probe_fails__then_it_prints_sidecar_hint(
        self,
        monkeypatch,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(
            translation_overrides={
                "enabled": True,
                "provider": "translategemma_local",
                "source_language": "zh",
                "target_language": "en",
            }
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.probe_translategemma_local_service",
            lambda **kwargs: (_ for _ in ()).throw(SecretError("sidecar unavailable")),
        )

        result = cli_runner.invoke(
            app,
            ["doctor", "--config", str(config_path)],
            env=_iflytek_env(),
        )

        assert result.exit_code == 1
        assert "sidecar unavailable" in result.output
        assert "local-translation serve" in result.output

    def test_when_translation_secret_is_missing__then_it_exits_non_zero(
        self,
        monkeypatch,
        tmp_cwd: Path,
        config_file_factory,
        cli_runner,
    ) -> None:
        backend = FakeAudioBackend()
        osc_transport = FakeOscTransport()
        config_path = config_file_factory(
            translation_overrides={
                "enabled": True,
                "provider": "deepl",
                "target_language": "en",
            }
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_audio_backend", lambda: backend
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.create_osc_chatbox_transport",
            lambda *, app_config, logger: osc_transport,
        )
        monkeypatch.setattr(
            "vrc_live_caption.cli.validate_translation_runtime",
            lambda **kwargs: (_ for _ in ()).throw(
                SecretError(
                    "DEEPL_AUTH_KEY not found. Add it to .env or set the environment variable."
                )
            ),
        )

        result = cli_runner.invoke(app, ["doctor", "--config", str(config_path)])

        assert result.exit_code == 1
        assert "DEEPL_AUTH_KEY not found" in result.output
