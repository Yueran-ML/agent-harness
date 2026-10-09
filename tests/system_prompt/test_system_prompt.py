"""The system prompt: the text of the spec, and the sections that plugins add."""

import pytest

import columbina.system_prompt
from columbina.kernel import Context, Next, RegistryConflictError
from columbina.system_prompt import (
    SYSTEM_PROMPT,
    SYSTEM_PROMPT_ASSEMBLE,
    Draft,
    Section,
    SystemPrompt,
)


@pytest.fixture
async def prompt(ctx: Context) -> SystemPrompt:
    await ctx.plugin(columbina.system_prompt)
    return ctx.use(SYSTEM_PROMPT)


async def test_without_sections_the_prompt_is_the_text_of_the_spec(
    ctx: Context, prompt: SystemPrompt
) -> None:
    assert await prompt.render(ctx, "You are helpful.") == "You are helpful."
    assert await prompt.render(ctx, "") == ""


async def test_sections_follow_the_text_of_the_spec_with_a_blank_line_between(
    ctx: Context, prompt: SystemPrompt
) -> None:
    prompt.sections.add(ctx, "files", Section("Read a file before you write it."))
    prompt.sections.add(ctx, "commands", Section("A command runs in a shell of its own."))

    assert await prompt.render(ctx, "You are helpful.") == (
        "You are helpful.\n\n"
        "A command runs in a shell of its own.\n\n"
        "Read a file before you write it."
    )


async def test_sections_come_by_their_numbers_and_then_by_their_names(
    ctx: Context, prompt: SystemPrompt
) -> None:
    prompt.sections.add(ctx, "b", Section("b zero"))
    prompt.sections.add(ctx, "z", Section("z minus", order=-5))
    prompt.sections.add(ctx, "a", Section("a ten", order=10))
    prompt.sections.add(ctx, "c", Section("c ten", order=10))
    prompt.sections.add(ctx, "a0", Section("a zero"))

    assert (await prompt.render(ctx, "base")).split("\n\n") == [
        "base",
        "z minus",
        "a zero",
        "b zero",
        "a ten",
        "c ten",
    ]


async def test_the_order_does_not_depend_on_when_a_section_was_added(ctx: Context) -> None:
    one, other = Context(), Context()
    try:
        await one.plugin(columbina.system_prompt)
        await other.plugin(columbina.system_prompt)
        for name in ("a", "b", "c"):
            one.use(SYSTEM_PROMPT).sections.add(one, name, Section(name))
        for name in ("c", "a", "b"):
            other.use(SYSTEM_PROMPT).sections.add(other, name, Section(name))

        first = await one.use(SYSTEM_PROMPT).render(one, "base")
        second = await other.use(SYSTEM_PROMPT).render(other, "base")

        assert first == second == "base\n\na\n\nb\n\nc"
    finally:
        await one.fiber.dispose()
        await other.fiber.dispose()


async def test_space_around_a_text_and_a_text_that_is_empty_leave_no_gap(
    ctx: Context, prompt: SystemPrompt
) -> None:
    prompt.sections.add(ctx, "a", Section("\n\n  first  \n"))
    prompt.sections.add(ctx, "b", Section("   "))
    prompt.sections.add(ctx, "c", Section("second"))

    assert await prompt.render(ctx, "  base\n") == "base\n\nfirst\n\nsecond"
    assert await prompt.render(ctx, "") == "first\n\nsecond"


async def test_a_section_leaves_with_the_plugin_that_added_it(
    ctx: Context, prompt: SystemPrompt
) -> None:
    class Adding:
        name = "adding"

        @staticmethod
        def apply(inner: Context, config: object) -> None:
            inner.use(SYSTEM_PROMPT).sections.add(inner, "extra", Section("extra"))

        inject = (SYSTEM_PROMPT,)

    loaded = await ctx.plugin(Adding)
    assert await prompt.render(ctx, "base") == "base\n\nextra"

    await loaded.dispose()

    assert await prompt.render(ctx, "base") == "base"


async def test_two_sections_cannot_have_one_name(ctx: Context, prompt: SystemPrompt) -> None:
    prompt.sections.add(ctx, "files", Section("one"))

    with pytest.raises(RegistryConflictError):
        prompt.sections.add(ctx, "files", Section("another"))


async def test_a_listener_is_handed_the_parts_and_may_answer_another_text(
    ctx: Context, prompt: SystemPrompt
) -> None:
    seen: list[Draft] = []

    async def shouting(draft: Draft, proceed: Next[Draft, str]) -> str:
        seen.append(draft)
        return (await proceed(draft)).upper()

    prompt.sections.add(ctx, "files", Section("Read first.", order=3))
    ctx.on(SYSTEM_PROMPT_ASSEMBLE, shouting)

    assert await prompt.render(ctx, "Be kind.") == "BE KIND.\n\nREAD FIRST."
    assert seen == [
        Draft(
            base="Be kind.",
            sections=(("files", Section("Read first.", order=3)),),
            text="Be kind.\n\nRead first.",
        )
    ]


async def test_a_listener_that_answers_no_text_is_an_error(
    ctx: Context, prompt: SystemPrompt
) -> None:
    async def silent(draft: Draft, proceed: Next[Draft, str]) -> str:
        return None  # pyright: ignore[reportReturnType]

    ctx.on(SYSTEM_PROMPT_ASSEMBLE, silent)

    with pytest.raises(TypeError, match="which is no text"):
        await prompt.render(ctx, "base")
