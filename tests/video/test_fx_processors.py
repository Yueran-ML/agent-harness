"""Protocol mirrors use supplied responses; no duplicate DSP or real engine here."""

from pathlib import Path

from columbina.video.engine import Engine
from support import fake_engine
from support.fake_engine import served


async def test_fx_readers_presets_and_input_noise_job(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        client = await Engine.attach(tmp_path / "film", program=str(fake_engine.__file__))
        fake.results["audio.fx_meters"] = {"playing": False, "readings": []}
        fake.results["audio.soft_clip_curve"] = {"revision": 7, "inputs": [0], "outputs": [0]}
        preset = {
            "id": "ap1",
            "name": "Gain",
            "payload": {"kind": "effect", "settings": {"kind": "gain", "gain_db": -3}},
        }
        fake.results.update(
            {
                "audio_presets.list": {"presets": [preset]},
                "audio_presets.save": preset,
                "audio_presets.update": preset,
                "audio_presets.delete": {"deleted": "ap1"},
            }
        )
        assert await client.fx_meters("t1") == {"playing": False, "readings": []}
        assert (await client.soft_clip_curve("t1", {"track": "a"}, 2, [0]))["revision"] == 7
        assert await client.audio_presets() == [preset]
        assert await client.save_audio_preset("Gain", preset["payload"]) == preset
        assert await client.update_audio_preset("ap1", preset["payload"]) == preset
        assert await client.delete_audio_preset("ap1") == {"deleted": "ap1"}
        fake.sound_found = {"profile": [0.1], "result": {"applied": {"revision": 8}}}
        learned = await client.noise_learn("t1", {"track": "a"}, 1, (0, 25), revision=7)
        assert learned["profile"] == [0.1]
        assert fake.sounds[-1] == (
            "audio.noise_learn",
            {
                "timeline": "t1",
                "target": {"track": "a"},
                "slot": 1,
                "range": {"from": 0, "to": 25},
                "revision": 7,
            },
        )
        assert client.latest_sound("t1", revision=8) is None
        await client.aclose()
