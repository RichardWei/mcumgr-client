# Existing-device compatibility

## Baseline

Original develop: 6012675db950503b854b9d9322be55ea5c1d9337.
Upstream: 92e2313cb0634af3f965a8668e5559993d1f65a2.
Working branch: develop-cpp-fii. The original develop branch is unchanged.

The rebase skipped the old staticlib, timeout and string-based error-code implementations because the new independent C ABI replaces them. The old merge node is not replayed by the ordinary rebase. The boot_version change, including its simulator fixture, is retained.

## Wire compatibility

Source comparison shows the serial 8-byte SMP header, big-endian lengths and group IDs, Base64 framing, CRC16/XMODEM, and existing image/reset command IDs are retained. Image upload retains data/image/off and the first-packet len/sha fields. This is strong evidence of compatibility for devices already accepting develop serial requests, not a hardware test result.

boot_version stays a mandatory String in image-list responses and JSON output. Missing or non-string values fail parsing, as on develop. New OS BootloaderInfo commands are separate and do not replace this field.

Upstream now supports UDP and BLE with SMP v2 version bits. Existing serial compatibility does not imply those transports or new commands are implemented by the device. The serial decoder still does not mask version bits from its opcode byte.

## Behavior changes requiring device testing

- Image chunk sizing is based on the transport's max_payload calculation; block boundaries can differ.
- Upload retries cover transport errors beyond the old exact timeout text; each transport call assigns a new sequence number.
- The CLI and new C ABI use upstream's 200 ms subsequent timeout convention, compared with old develop's 4000 ms. Callers can explicitly use 4000.
- Empty image files are rejected by the new ABI.
- Upstream error helpers examine top-level rc but do not comprehensively support SMP v2 nested err/group/rc responses. Raw custom commands return the response body without interpreting it.
- Upstream serial/UDP payload-limit arithmetic and framing overhead are retained unchanged; CI simulation cannot certify the device's receive-buffer limits. Use conservative MTUs and test real firmware.

## Validation and acceptance

CI runs upstream and FFI tests, simulated serial image-list/upload tests, loopback UDP required-field tests, and independent native C/C++ consumer link/run tests. This verifies host behavior only.

Hardware acceptance still requires the user's device: query image state and boot_version; upload first/subsequent/final chunks; force a lost response and check repeated-offset/new-sequence handling; confirm/erase/reset as authorized; check observed response latency against the selected timeout. No physical device operation is performed automatically by CI.

