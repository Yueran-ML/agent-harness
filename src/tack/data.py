"""The base of the plain data that Tack passes around and writes down."""

from pydantic import BaseModel, ConfigDict


class Data(BaseModel):
    """Plain data: immutable, and strict about unknown fields."""

    model_config = ConfigDict(frozen=True, extra="forbid")
