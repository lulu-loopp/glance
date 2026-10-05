# Building the modules

These are the sources of the five PawnIO modules Glance carries
(`AMDFamily17`, `IntelMSR`, `LpcIO`, `SmbusPIIX4`, `SmbusI801`), unmodified from
[PawnIO.Modules release 0.2.11](https://github.com/namazso/PawnIO.Modules/tree/0.2.11),
with the include files they need.

## Compiling

Upstream compiles them (its `.github/workflows/ci.yml` at that tag) with the
Pawn compiler 4.1.7152 it ships in `_pawn/` (`pawn-4.1.7152-1.el9.x86_64.rpm`,
and its source `pawn-4.1.7152-1.el9.src.rpm`), on Oracle Linux 9:

```
dnf install -y pawn-4.1.7152-1.el9.x86_64.rpm
for f in ./*.p; do pawncc "$f" -iinclude -C64 '-;+' '-(+' -p; done
```

Each `name.p` gives `name.amx`.

## What Glance loads

Glance's `pawnio-modules/name.bin` is upstream's released module: a
container of a 4-byte little-endian signature length, an RSA signature, and
then the compiled `.amx`. The released PawnIO driver checks that signature
against namazso's key and loads only modules signed with it
([`vm.cpp`](https://github.com/namazso/PawnIO/blob/2.2.0/PawnIO/src/vm.cpp)).

So a changed module runs only on a PawnIO driver built with
`PAWNIO_UNRESTRICTED`, which skips the check (and, being unsigned by
Microsoft, needs Windows' test-signing mode). For such a driver, build the
container with a zero signature length:

```
python -c "import sys; d=open(sys.argv[1],'rb').read(); open(sys.argv[2],'wb').write((0).to_bytes(4,'little')+d)" name.amx name.bin
```

then put `name.bin` in `pawnio-modules/` and build Glance from its source
(`cargo build --release`): the modules are compiled into the executable.

Licence: GNU Lesser General Public License 2.1: `pawnio-modules/COPYING` in
Glance's source, `PawnIO-Modules-LGPL-2.1.txt` beside this folder in an
installed Glance.
