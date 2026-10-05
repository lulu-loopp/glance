# Building the modules

These are the sources of the four PawnIO modules Glance carries
(`AMDFamily17`, `LpcIO`, `SmbusPIIX4`, `SmbusI801`), unmodified from
[PawnIO.Modules release 0.2.11](https://github.com/namazso/PawnIO.Modules/tree/0.2.11),
with the include files they need.

They are compiled exactly as upstream compiles them (its
`.github/workflows/ci.yml` at that tag), with the Pawn compiler 4.1.7152 that
upstream ships in `_pawn/` (`pawn-4.1.7152-1.el9.x86_64.rpm`, and its source
`pawn-4.1.7152-1.el9.src.rpm`), on Oracle Linux 9:

```
dnf install -y pawn-4.1.7152-1.el9.x86_64.rpm
for f in ./*.p; do pawncc "$f" -iinclude -C64 '-;+' '-(+' -p; done
```

Each `name.p` gives `name.amx`; Glance's `pawnio-modules/name.bin` is that
file. To run Glance with modules you have changed, put your `.amx` files in
place of the `.bin` files and build Glance from its source (`cargo build
--release`); the modules are compiled into the executable.

Licence: GNU Lesser General Public License 2.1 (`../COPYING`).
