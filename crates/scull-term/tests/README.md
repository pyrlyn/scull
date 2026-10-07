# scull-term conformance fixtures

`conformance.rs` replays every case in `fixtures/*.txt` against a fresh
`Terminal` and compares what the case names: screen text, cursor, cell
styles, scrollback length and, from T7.2, reply bytes. A headless core can
check all of these, so the subset is the part of esctest2 and vttest that
needs no pixels, no timing and no host interaction.

## Licence note

esctest2 (<https://github.com/ThomasDickey/esctest2>, `LICENSE` at commit
`2798f12149a19c3295e9b4853ab2da4b2eff1b2b`, checked 2026-09-13) is GPL-2.0
only. That is not compatible with Scull's GPL-3.0-or-later, so no esctest2
code or data is copied. Each case here is our own fixture, written from the
behaviour an esctest2 test describes, and names that test in its `source:`
line so a reader can compare. vttest (<https://invisible-island.net/vttest/>)
screens are used the same way: the expected behaviour, not the program.

Behaviour choices beyond those tests follow xterm's ctlseqs
(<https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>) and the DEC
manuals on vt100.net (<https://vt100.net/docs/>).

## Fixture format

```text
# a comment
=== name of the case, written as the claim it proves
source: where the expected behaviour comes from
size: 10x5            # COLSxROWS, default 10x5
scrollback: 0         # history rows kept, default 0
input: \e[3;4H        # may repeat; the parts are concatenated
cursor: 3,4           # 1-based ROW,COL, then " wrap" if a wrap is pending
history: 2            # rows in the scrollback
style: 1,1 bold fg=1  # the style of the cell at 1-based ROW,COL
screen:
|row one   |
|row two   |
```

Input escapes: `\e` ESC, `\r`, `\n`, `\t`, `\b` BS, `\\`, `\s` space and
`\xHH` for any byte (multi-byte UTF-8 is spelled byte by byte). In screen
rows an empty cell is `.`, the second half of a wide character is skipped
and a grapheme cluster is its text. A style is the attributes in the order
`bold dim italic blink inverse hidden strike overline`, then `fg=`, `bg=`,
`ul=` (underline shape) and `ulc=` (underline colour); a colour is a palette
index or `#rrggbb`, and a cell with none of them is `default`. Every case
must check at least one thing.

## Cases (T7.1)

| File | Cases | Sources |
| --- | --- | --- |
| `cursor.txt` | CUP, HVP, CUU, CUD, CUF, CUB, CHA, HPA, VPA, CNL, CPL, HPR, VPR, BS, CR, LF, VT/FF; defaults, zero parameters, clamping, scroll-region stops | esctest2 `cup.py`, `hvp.py`, `cuu.py`, `cud.py`, `cuf.py`, `cub.py`, `cha.py`, `hpa.py`, `vpa.py`, `cnl.py`, `cpl.py`, `hpr.py`, `vpr.py`, `bs.py`, `cr.py`, `lf.py`; xterm ctlseqs C0 |
| `erase.txt` | DECALN, EL 0/1/2, ED 0/1/2/3, ECH, erase in the pending wrap, background colour erase | esctest2 `decaln.py`, `el.py`, `ed.py`, `ech.py`; xterm ctlseqs BCE |
| `insert_delete.txt` | ICH, DCH, IL, DL; counts past the line or screen, scroll-region limits, wide-character repair | esctest2 `ich.py`, `dch.py`, `il.py`, `dl.py` |
| `scroll.txt` | SU, SD, IND, RI, NEL, LF at margins, DECSTBM (homing, invalid, default, clamped, zero), scrollback feed | esctest2 `su.py`, `sd.py`, `ind.py`, `ri.py`, `nel.py`, `lf.py`, `decstbm.py` |
| `tabs.txt` | HT, HTS, TBC 0/3, CHT, CBT | esctest2 `hts.py`, `tbc.py`, `cht.py`, `cbt.py`, `decset.py` (tab does not wrap); vttest menu 2 |
| `sgr.txt` | attributes on and off, 16/256/truecolour in semicolon and colon forms, out-of-range colours, underline shapes and colour | esctest2 `sgr.py`; xterm ctlseqs SGR; ITU T.416 colon forms |
| `wrap.txt` | pending wrap, wrap, CR and BS in the pending wrap, wrap at the bottom into the scrollback, REP | esctest2 `decset.py` (DECAWM), `bs.py`, `rep.py`; vttest menu 1 |
| `unicode.txt` | wide characters, wrapping and overwriting them, combining marks, ZWJ sequences, VS16 without mode 2027, controls ending a cluster | Unicode UAX #11, UAX #29, UTS #51; foot and Alacritty for the leading spacer |

Not covered here: anything needing DECSLRM, DECOM, IRM, DECAWM off or
replies arrives with T7.2. Tests that need a real host (DECCOLM, 132-column
switching, printer and locator reports, checksums via DECRQCRA) are out of
scope for a headless core.
