"""Writes src/simplified.tsv: each text in traditional Chinese characters and the same in simplified ones, a line
each ("traditional<TAB>simplified"), its phrases first, then its characters, each with the first of what OpenCC writes
it as (its own t2s conversion's choice). Every one keeps its number of characters.

From OpenCC's TSPhrases.txt and TSCharacters.txt (data/dictionary, Apache License 2.0, see
licenses/THIRD-PARTY.txt):

    python scripts/simplified.py path/to/TSPhrases.txt path/to/TSCharacters.txt
"""
import sys

seen = set()
lines = []
for path in sys.argv[1:3]:
    for line in open(path, encoding='utf-8'):
        if line.startswith('#') or not line.strip():
            continue
        traditional, simplified = line.rstrip('\n').split('\t', 1)
        first = simplified.split(' ')[0]
        if len(first) != len(traditional):
            sys.exit(f'{traditional} -> {first}: not as many characters')
        # (A phrase written as it is stays: it keeps its characters from being written otherwise one by one, 乾隆's
        # 乾 not 干. A character written as it is, alone, changes nothing.)
        if traditional in seen or (traditional == first and len(traditional) == 1):
            continue
        seen.add(traditional)
        lines.append(f'{traditional}\t{first}\n')
# (Its notice, as the Apache License asks of a file changed from the work: lines without a tab, which Glance skips.)
NOTICE = """# Derived from OpenCC (Open Chinese Convert) 1.4.2, https://github.com/BYVoid/OpenCC
# Copyright Carbo Kuo (BYVoid) and the OpenCC contributors. Licensed under the Apache License, Version 2.0:
# licenses/OpenCC-Apache-2.0.txt, see licenses/THIRD-PARTY.txt.
# Changed by Glance (scripts/simplified.py): data/dictionary/TSPhrases.txt and TSCharacters.txt joined, phrases first,
# each text kept with the first of what it is written as only, characters written as they are left out.
"""
with open('src/simplified.tsv', 'w', encoding='utf-8', newline='') as out:
    out.write(NOTICE)
    out.writelines(lines)
print(len(lines), 'kept')
