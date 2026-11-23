# foxden-8bitdo

Settings and light rings of 8BitDo controllers, worked out from 8BitDo Ultimate Software V2 1.38
for Windows (its `8BitDoAdvance.dll` and the structures its .NET app passes to it). No 8BitDo pad
has been driven by this code yet.

## Channel

The vendor HID collection (usage page `FF7A` or `FFA0`, usage 1, 64-byte reports). Requests are
output report `81`, answers input report `02`.

```
81 04 <header 16> <payload ≤45>        Pro 3, HitBox, HitBox 2, Ultimate 2 (both)
81 <n+1> 04 <header 16> <payload>      Pro 2, Ultimate Bluetooth, Ultimate 2.4G, Ultimate PC
```

Header, little-endian: `cmd u16, arg u16, len u16, crc u16, total u32, offset u32`. `crc` is
CRC-16/MODBUS of the payload on the pads that check it, 0 on the others (then `len` is a u32).
Answers: `02 04 04 00 <cmd u16> <count u16> ... <data from byte 18>`.

| cmd | what |
| --- | --- |
| `0001` | write `len` bytes of the config block at `offset` (`total` = block size) |
| `0002` | read up to `len` bytes at `offset` |
| `0006` | store what was written (`arg` = `0123`); the pad answers within about 4 s |
| `0007` | input reports off (`arg` 0) and on (1) around config writes |
| `0040` | light theme now (`arg`): 2 follows sticks, 3 spins, 4 custom colours, 5 off |
| `0041` | current light theme |
| `0100` | identity: the pad's own product id at data bytes 4-5, also behind a receiver |
| `0121` | battery level, data byte 0 (bit 7 charging) |

## Config block

One block per pad, three profiles (two on the HitBox 2): profile names (UTF-16 big-endian),
vibration strength per motor, stick and trigger ranges, a feature word (invert axes, swap sticks or
triggers, D-pad mode, dead zone off, sleep...), 20-24 button maps as output-button masks, XInput
vibration ranges, motion aiming (Ultimate 2), light ring colours (Ultimate 2), macros, and HitBox 2
lighting and combos. Offsets are in `model.rs`; each layout is checked against the block size the
vendor library's exports declare (`writeUltimate2@1592` and so on).

A section the app has set carries the marker `20200911`. The block's CRC field is CRC-16/MODBUS
of the block as it stands, old CRC included, the way the vendor app computes it.

## Light rings

The Ultimate 2 keeps its 24 custom colours (12 per stick, `0x00RRGGBB`) in the config block and has
no live mode. The lighting driver in `foxden-hid` finds the pad but writes nothing unless the
environment variable `FOXDEN_8BITDO_WRITE` is `1`. Then it writes colours only after they have held
still for 1.5 s, at most every 3 s, into the profile in use, and switches the theme to custom;
animated effects are not sent. Before its first write it checks the receiver has its pad and reads
the profile and theme, and it puts the old theme back when it lets go.

## Not done

Firmware updates, macro step recording (`WriteMacro`/`EraseMacro`), HitBox 2 lighting fields,
8BitDo keyboards and mice (other protocols in the same library), Ultimate 3 (its own SDK).
