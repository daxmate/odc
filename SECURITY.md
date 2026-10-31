# Security policy

## Supported versions

ODC is at 0.x; only the latest minor release receives fixes.

| Version | Supported |
| --- | --- |
| 0.1.x | Yes |
| < 0.1 | No |

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Use GitHub's
private reporting channel instead:

<https://github.com/daxmate/odc/security/advisories/new>

Include:

* the ODC version (`odc --version`), your platform, and how you built it;
* a minimal reproduction (ideally a small repository or a hex dump plus the
  command that fails);
* the observed and expected behaviour, and any error code (`ODC-E0xx`) shown.

You can expect an acknowledgement within a few days and a fix or mitigation
plan once the report is triaged. Please give us a reasonable window to release
a fix before public disclosure.

## Scope

In scope:

* memory-safety or panic bugs reachable from untrusted `.odc` files, blocks,
  CSV/JSONL input, or patch files;
* denial of service through malformed input that is not bounded by the
  specification (for example unbounded allocation on a crafted length);
* path traversal or arbitrary file writes triggered by a crafted repository.

Out of scope: vulnerabilities in third-party crates (please report them
upstream, though we appreciate a heads-up), and problems that require an
already-compromised machine.

## Hardening notes

The core crate forbids `unsafe` (`unsafe_code = "forbid"` in the workspace
lints). All decoders validate lengths before allocating and return
`Error` values instead of panicking; the negative-test suite exercises
truncated blocks, bad magic, duplicate keys, overlong varints and corrupted
indexes.
