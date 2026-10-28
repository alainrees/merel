# merel-signal

Merel's link to Signal: a small program that links as a secondary device of your Signal
account, as Signal Desktop does, and passes messages to Merel. It is built on
[presage](https://github.com/whisperfish/presage) and Signal's own libsignal.

Those libraries are licensed AGPL-3.0, so this program is too. Merel starts it as a separate
program and talks to it over standard input and output (one JSON object per line); Merel itself
stays MIT OR Apache-2.0.

```text
MEREL_SIGNAL_KEY=… merel-signal link <dir> <device name>
  {"type":"link","url":"sgnl://linkdevice?…"}   show as a QR code, scan with Signal on the phone
  {"type":"linked","number":"+31…"}

MEREL_SIGNAL_KEY=… merel-signal run <dir>
  {"type":"message","thread":…,"thread_name":…,"sender_name":…,"mine":false,"timestamp":…,"text":…}
  {"type":"synced"}
  ← {"type":"send","thread":…,"text":…}
  {"type":"sent","thread":…,"timestamp":…}
```

`<dir>` holds the encrypted store (SQLCipher, key from `MEREL_SIGNAL_KEY`). A linked device only
gets messages from the moment it is linked on; Signal does not hand it the history.

Building needs `protoc` (`apt install protobuf-compiler`, `brew install protobuf`).
