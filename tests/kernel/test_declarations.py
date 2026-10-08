from dataclasses import FrozenInstanceError

import pytest

from tack.kernel import (
    BailEvent,
    CompositionError,
    EmitEvent,
    Event,
    EventModeError,
    InactiveEffectError,
    InvalidPluginError,
    KernelError,
    ParallelEvent,
    SerialEvent,
    ServiceConflictError,
    ServiceKey,
    ServiceNotInjectedError,
    ServiceUnavailableError,
    WaterfallEvent,
)


def test_service_keys_with_the_same_name_are_the_same_slot() -> None:
    assert ServiceKey[int]("clock") == ServiceKey[int]("clock")
    assert hash(ServiceKey[int]("clock")) == hash(ServiceKey[int]("clock"))
    assert ServiceKey[int]("clock") != ServiceKey[int]("timer")


def test_service_key_shows_its_name() -> None:
    assert repr(ServiceKey[int]("clock")) == "ServiceKey(name='clock')"


def test_service_key_is_immutable() -> None:
    key = ServiceKey[int]("clock")
    with pytest.raises(FrozenInstanceError):
        key.name = "timer"  # pyright: ignore[reportAttributeAccessIssue]


@pytest.mark.parametrize(
    ("kind", "mode"),
    [
        (EmitEvent, "emit"),
        (ParallelEvent, "parallel"),
        (SerialEvent, "serial"),
        (BailEvent, "bail"),
        (WaterfallEvent, "waterfall"),
    ],
)
def test_each_event_kind_has_a_fixed_dispatch_mode(kind: type[Event], mode: str) -> None:
    event = kind("demo/thing")
    assert event.mode == mode
    assert event.name == "demo/thing"
    assert repr(event) == f"{kind.__name__}('demo/thing')"


def test_event_declarations_accept_type_parameters() -> None:
    assert EmitEvent[int]("demo/count").name == "demo/count"
    assert WaterfallEvent[str, int]("demo/measure").mode == "waterfall"


@pytest.mark.parametrize(
    "error",
    [
        InactiveEffectError,
        ServiceConflictError,
        ServiceUnavailableError,
        ServiceNotInjectedError,
        EventModeError,
        InvalidPluginError,
        CompositionError,
    ],
)
def test_kernel_errors_share_a_base_class(error: type[Exception]) -> None:
    assert issubclass(error, KernelError)


def test_misuse_errors_are_also_type_errors() -> None:
    assert issubclass(EventModeError, TypeError)
    assert issubclass(InvalidPluginError, TypeError)
