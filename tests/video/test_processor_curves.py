"""Read-only processor request mirrors, with supplied DSP reply fixtures."""

from pathlib import Path

from columbina.video.engine import Engine
from support import fake_engine
from support.fake_engine import served


async def test_static_processor_curves_preserve_revision_and_history(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        client = await Engine.attach(tmp_path / "film", program=str(fake_engine.__file__))
        fake.results["audio.eq_response"] = {
            "revision": 7,
            "sample_rate": 48000,
            "frequencies": [100, 1000],
            "gain_db": [1.5, 3.0],
        }
        fake.results["audio.dynamics_curve"] = {
            "revision": 7,
            "inputs_db": [-60, -20, 0],
            "outputs_db": [-60, -20, -7.5],
        }
        eq = await client.eq_response("t1", {"track": "A1"}, [100, 1000])
        dy = await client.dynamics_curve("t1", {"bus": "B1"}, [-60, -20, 0])
        assert eq["gain_db"] == [1.5, 3.0]
        assert dy["outputs_db"] == [-60, -20, -7.5]
        assert eq["revision"] == dy["revision"] == 7
        assert fake.sounds == [
            (
                "audio.eq_response",
                {"timeline": "t1", "target": {"track": "A1"}, "frequencies": [100, 1000]},
            ),
            (
                "audio.dynamics_curve",
                {"timeline": "t1", "target": {"bus": "B1"}, "inputs_db": [-60, -20, 0]},
            ),
        ]
        assert fake.revision == 0
        assert not fake.steps
        assert not fake.transactions
        await client.aclose()
