# Example plugins for diroxide

This is the diro-examples pack: the example plugins and script commands that come with diroxide, as one optional component. diroxide downloads it only after you say yes, checks every file against the size and SHA-256 its list pins, and unpacks it into the components folder of its state folder.

## What is here

- `plugins/`: one folder per example, each with its `plugin.toml` or its script, and a short note on what it does and what it needs.

## Installing never enables

A plugin from this pack does nothing until you enable it. Enabling it shows what it may do and starts on No. Read an example before you enable it: each is small on purpose.

## Removing

`diro components remove diro-examples` lists every file the pack put there and removes those only. A file you added to the folder stays.

## License

Apache-2.0 OR MIT, the same as diroxide.
