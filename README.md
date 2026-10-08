# Tack

Tack is a plugin-based agent harness written in Python. Every capability is a plugin:
the model adapter, the tool registry, the session log, and the agent loop itself.

**Status:** early development. Nothing is usable yet.

Tack 是一个用 Python 写的插件化 Agent harness。模型适配、工具注册、会话日志和 Agent
循环都是插件。项目处于早期开发阶段，目前还不能使用。

## Development

Tack uses [uv](https://docs.astral.sh/uv/) for environments and needs Python 3.11 or newer.

```bash
uv sync
uv run ruff check .
uv run ruff format --check .
uv run pyright
uv run coverage run -m pytest
uv run coverage report
```

Design notes live in `docs/plans/`.

## License

MIT. See [LICENSE](LICENSE).
