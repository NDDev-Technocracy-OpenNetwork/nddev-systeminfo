"""Read the real native v1 provider, including its counter warm-up boundary."""
import sys

import rldyour_sysinfo

for count, sample in enumerate(rldyour_sysinfo.samples(interval=0, path=sys.argv[1]), start=1):
    assert sample["v"] == 1
    if count == 2:
        assert sample["cpu"]["usage"] is not None
        assert sample["memory"]["used"] is not None
        break
else:
    raise SystemExit("provider closed before two samples")
print("PASS: Python SDK reads the real native provider")
