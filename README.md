# Event Assembler Formatting Suite R

This repo contains additional utilities (or rewrites of existing ones) for
formatting different kinds of data for use by Event Assembler in GBA ROMhacking.

In particular, this repository contains the following:

- [gbalz77](gbalz77/), a library for GBA-formatted lz77 compression, along with
  its [accompanying binary](bin/gbalz77tool/)
- [tilemage](tilemage/), a library for manipulating GBA images, along with its
  [accompanying binary](bin/tilemage/).
- [mar2dmp](bin/mar2dmp/)

and other WIPs.

To use the libraries, use the following lines in your `Cargo.toml`:

```toml
gbalz77 = { git = "https://github.com/CT075/cam-formatting-suite" }
tilemage = { git = "https://github.com/CT075/cam-formatting-suite" }
```
