# NDDev Systeminfo Python client

Typed, dependency-free client for the local JSON protocol exposed by the
`nddev-systeminfod` Rust provider. The import and package identity remain
`rldyour_sysinfo` / `rldyour-sysinfo` for existing consumers.

```python
from rldyour_sysinfo import samples

for sample in samples(interval=2):
    print(sample["gpu"])
```

`interval` is the cadence in seconds, 0–60; `0` selects realtime, a 500 ms
tick. The command `rldyour-sysinfo --once` prints one sample as JSON.

Linux and macOS. CPython on Windows builds without `socket.AF_UNIX`, so this
client cannot reach the provider there. Use the native Rust client or
`nddev-systeminfo sample` on Windows.

Install the daemon from the
[main repository](https://github.com/NDDev-Technocracy-OpenNetwork/nddev-systeminfo).
Qualified wheels and source distributions accompany its GitHub releases.
