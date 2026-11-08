Historical qualification record; current contracts, issues and releases are authoritative.

# Native analysis and resource observations

PR #59 implements #55–#58: passive adapter details, logical cores, bounded
one-hour recording, shared Rust analysis and native metric cells/windows.
Ordinary local v1 remains compatible. The independent view schema is versioned;
native surfaces consume bounded Rust projections instead of calculating their
own statistics. Website navigation uses the product metadata.

Linux x64 release observations compare immutable v0.3.4 with source
`f8dc514959c167146c58e9e5efcf8419528b0d04`. The binaries' SHA-256 values are
`b5db93f1028c2e9fa22a72c885f6f67a85ba86cda18e25e56be1e5186f37db34`
and `44bc665aadd3d8c964dbfb0d45fdc2a8e935a1fff2f31a228fe9d77cd38a0d47`.
Both use explicit file configuration, realtime ordinary v1 demand, no recording,
ten seconds active and ten seconds idle after the thirty-second release period.

| Configuration | Active PSS, old/new KiB | Released PSS, old/new KiB | First frame, old/new ms |
| --- | --- | --- | --- |
| GPU disabled | 940 / 1052 | 941 / 1051 | 1.51 / 1.88 |
| GPU enabled | 26185 / 25736 | 5816 / 5823 | 21.31 / 8.76 |

The non-GPU case has a visible 112 KiB footprint increase. Active samples
advanced zero or one CPU tick in ten seconds; idle observations advanced zero.
This is one-host evidence with timer resolution and driver/cache effects,
not a universal latency or zero-CPU guarantee. Two threads serve GPU-disabled
demand; three serve active GPU demand.

The same new binary then recorded for 3605 seconds at requested 500 ms cadence.
It received 7191 frames. Replay returned 7180 records, with the oldest sample
3599794 ms old: the expired start had been removed. PSS grew from 25810 KiB to
32335 KiB; threads stayed at three and descriptors at twenty. After disconnect
and idle release, PSS was 12549 KiB and descriptors ten. The standalone process
retained allocator pages after freeing history; socket-activated production
providers can exit when idle. No history files were created. The source predates
later view-contract/UI fixes; it is not represented as the final release binary.

Native acceptance run 42521560759 at
`5ee9f5cb1fca3713ab7486cae9ccace923485784` passed all thirteen gates.
Its captures were downloaded and reviewed. Separate observations deliberately
name different measurement types:

- AppKit ARM: status-bar RSS 43968 KiB; open-window RSS 82096 KiB plus a
  6176 KiB reader. Pausing removed that child. Intel observations were
  23904 KiB, 49680 KiB and a 2396 KiB reader respectively. These are process
  RSS, not physical footprint or Linux PSS.
- GTK in headless native CI: open-window PSS 226029 KiB plus a 3344 KiB reader.
  Pause removed the reader, and closing the window removed the UI process.
  The toolkit/renderer footprint is significant and depends on the runtime;
  no global renderer or driver setting was changed to obtain a smaller number.
- Win32 control-surface projection: open working set approximately 24–25 MB,
  private commit approximately 2.7–3.7 MB. Five create/close cycles returned
  USER handles to four; GDI handles settled at twelve/thirteen with locale
  font caches. This test does not measure the full production reader worker.

Later Windows accessibility qualification exposes status/statistics as native
STATIC labels, alongside standard buttons and choices. A finite lifecycle run
does not prove leak freedom. Actual release qualification and device deployment
are tracked in #55; unsupported readings remain null.
