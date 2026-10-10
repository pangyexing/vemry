# Security

vemry is a local-first desktop application: it works without a server or an
account, and has no telemetry. The security-relevant surface is the code that
decides what the AI model is allowed to read and write — the privacy zones in
`crates/vemry-core`, the permission gate in `crates/vemry-agent`, the script
sandbox in `crates/vemry-script` — plus the update pipeline, the embedded
browser, and the client side of the optional vemry account (sign-in, the device
token, the hosted relay and the phone inbox, in `src-tauri/src/account.rs` and
`src-tauri/src/inbox.rs`).

## Reporting

Report a vulnerability privately to **security@vemry.app** with "vemry
security" in the subject. Include the version (the Settings dialog shows it next to "Check for updates"), the platform,
and steps to reproduce; a proof of concept is welcome, a full exploit is not
needed.

You will get an acknowledgement within a week. Fixes ship as an ordinary release;
the release note credits the reporter unless they ask otherwise.

## What this is not

There is **no bug bounty** and no paid programme of any kind. Reports generated
by automated scanners or language models without a working reproduction will be
closed without reply.

Out of scope: anything that requires the attacker to already control the vault
folder or the machine, issues in the model provider's service, and the content
of web pages loaded in the embedded browser (which runs the platform WebView with
its own security model).
