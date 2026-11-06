"""Wire mirrors only; the real engine tests validate the parameter catalogue and audio."""

from pathlib import Path

from columbina.video.engine import Engine
from support import fake_engine
from support.fake_engine import served


async def test_automation_session_requests_do_not_edit_project(tmp_path: Path) -> None:
    async with served(tmp_path / "film") as fake:
        client = await Engine.attach(tmp_path / "film", program=str(fake_engine.__file__))
        assert await client.automation_playback("t1", False) == {"on": False}
        assert await client.automation_parameters("t1", {"vca": "V1"}) == {
            "revision": 0,
            "of": {"vca": "V1"},
            "parameters": [],
        }
        assert await client.automation_playback("t1", True) == {"on": True}
        assert fake.looked[-3:] == [
            ("automation.playback", {"timeline": "t1", "on": False}),
            ("automation.parameters", {"timeline": "t1", "of": {"vca": "V1"}}),
            ("automation.playback", {"timeline": "t1", "on": True}),
        ]
        assert fake.revision == 0
        assert not fake.steps
        assert not fake.transactions
        await client.aclose()
