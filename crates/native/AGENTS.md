# Native OS access

Read [README.md](README.md) and the [root contract](../../AGENTS.md).

This is the sole unsafe Rust boundary approved in decision 0002. Document pointer validity, handle ownership and drop behavior. Never intentionally wake suspended hardware or request elevated sensor access. Run lifecycle checks on the target OS or its native CI runner.
