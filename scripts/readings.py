"""Writes src/search/readings.bin: for each CJK character from U+3400 to U+9FFF, its Mandarin readings in pinyin
without tones (ü written v), the most usual first, for finding a file by its name spelt in pinyin.

From the Unicode Character Database's Unihan_Readings.txt (kMandarin first, then kHanyuPinyin, kXHC1983, kTGHZ2013;
Unicode License v3, see licenses/THIRD-PARTY.txt):

    python scripts/readings.py path/to/Unihan_Readings.txt

The file: the number of syllables (u16), each syllable (its length, u8, then its letters); then, for each character in
order, how many readings it has (u8); then every character's readings in order, each a syllable's place (u16). All
numbers little-endian.
"""
import struct
import sys
import unicodedata

FIRST, LAST = 0x3400, 0x9FFF
FIELDS = ['kMandarin', 'kHanyuPinyin', 'kXHC1983', 'kTGHZ2013']


def toneless(reading):
    """A reading without its tone, ü written v: 'lǜ' -> 'lv'."""
    letters = []
    for c in unicodedata.normalize('NFD', reading.strip().lower()):
        if c == '̈' and letters and letters[-1] == 'u':
            letters[-1] = 'v'
        elif 'a' <= c <= 'z':
            letters.append(c)
    return ''.join(letters)


by_field = {}
for line in open(sys.argv[1], encoding='utf-8'):
    if line.startswith('#') or not line.strip():
        continue
    code, field, value = line.rstrip('\n').split('\t', 2)
    if field not in FIELDS:
        continue
    point = int(code[2:], 16)
    if not FIRST <= point <= LAST:
        continue
    # kHanyuPinyin and kXHC1983 give "where:reading,reading"; kTGHZ2013 "where:reading"; kMandarin readings alone.
    readings = []
    for part in value.split(' '):
        readings.extend(part.split(':', 1)[-1].split(','))
    by_field.setdefault(point, {})[field] = [toneless(reading) for reading in readings]

syllables, places = [], {}
counts, stream = [], []
for point in range(FIRST, LAST + 1):
    readings = []
    for field in FIELDS:
        for reading in by_field.get(point, {}).get(field, []):
            if reading and reading not in readings:
                readings.append(reading)
    for reading in readings:
        if reading not in places:
            places[reading] = len(syllables)
            syllables.append(reading)
        stream.append(places[reading])
    counts.append(len(readings))

out = 'src/search/readings.bin'
with open(out, 'wb') as f:
    f.write(struct.pack('<H', len(syllables)))
    for syllable in syllables:
        f.write(struct.pack('<B', len(syllable)) + syllable.encode('ascii'))
    f.write(bytes(counts))
    f.write(b''.join(struct.pack('<H', place) for place in stream))
print(out, len(syllables), 'syllables,', sum(1 for count in counts if count), 'characters,', len(stream), 'readings')
