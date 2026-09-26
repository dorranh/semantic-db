"""Regenerate the SemanticDB SVG asset family with Python 3 (standard library only)."""

from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]
WORD = json.loads((ROOT / 'source/wordmark-path.json').read_text())
INK = '#252e33'
PAPER = '#fbf8f0'

# Hand-drawn cubic Bezier reconstruction of the approved concept, in its
# original 1254-pixel coordinate system. Holes are transparent, not white paint.
OUTER = '''M 266 998
C 214 971 181 918 182 853
C 180 774 222 721 283 662
C 344 603 391 569 426 509
C 455 459 464 395 478 337
C 496 260 554 209 627 209
C 700 208 750 250 770 329
C 788 400 799 462 831 522
C 866 590 909 639 974 690
C 1039 740 1075 785 1074 852
C 1074 923 1027 975 958 1000
C 873 1032 771 1026 682 1008
C 589 989 519 958 469 911
C 438 882 399 862 357 862
C 302 861 260 884 244 921
C 231 949 239 978 266 998 Z'''
CENTER = '''M 305 696
C 352 628 393 586 448 541
C 513 486 566 440 633 440
C 701 438 755 477 790 535
C 814 575 829 625 858 662
C 885 697 923 720 965 736
C 899 733 834 747 793 773
C 762 793 752 813 758 844
C 764 872 776 895 759 919
C 742 946 706 961 674 960
C 613 962 557 930 519 884
C 492 851 476 810 454 769
C 435 731 409 704 374 694
C 350 686 329 688 305 696 Z'''
FACE = '''M 578 281
C 607 279 645 279 673 282
C 711 284 737 309 738 342
C 740 377 712 408 678 409
L 578 409 C 544 409 517 382 516 350
C 514 315 541 285 578 281 Z'''
HUMAN = '''M 343 725
C 379 724 408 751 409 786
C 410 820 381 847 346 847
C 309 848 279 823 279 790
C 277 755 306 727 343 725 Z'''
DB_TOP = '''M 790 815
C 788 791 845 770 909 769
C 976 767 1035 783 1037 807
C 1039 832 982 853 918 855
C 851 858 793 840 790 815 Z'''
DB_BOTTOM = '''M 1037 867
C 1010 890 965 899 920 899
C 883 899 848 892 826 890
C 806 888 794 897 793 911
C 790 937 829 956 878 959
C 936 963 989 948 1016 920
C 1030 906 1039 887 1037 867 Z'''


def mark(x=0, y=0, size=1024):
    scale = size / 1024
    return f'''<g transform="translate({x:g} {y:g}) scale({scale:g})">
  <g transform="translate(-116 -103)">
    <path fill-rule="evenodd" d="{' '.join((OUTER, CENTER, FACE, HUMAN, DB_TOP, DB_BOTTOM)).replace(chr(10), ' ')}"/>
    <circle cx="577.5" cy="349" r="23.5"/>
    <circle cx="676.5" cy="349" r="23.5"/>
  </g>
</g>'''


def wordmark(x, y, height):
    scale = height / WORD['height']
    return f'<path transform="translate({x:g} {y:g}) scale({scale:g})" d="{WORD["path"]}"/>'


def svg(name, width, height, body, color=INK, background=None, extra=''):
    bg = f'<rect width="{width:g}" height="{height:g}" fill="{background}"/>\n' if background else ''
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width:g}" height="{height:g}" viewBox="0 0 {width:g} {height:g}" role="img" aria-label="{name}">
<title>{name}</title>
{extra}{bg}<g fill="{color}">
{body}
</g>
</svg>
'''


def build():
    word_ratio = WORD['width'] / WORD['height']
    horizontal_word_height = 112
    horizontal_width = 280 + word_ratio * horizontal_word_height + 28
    stacked_word_height = 92
    stacked_width = max(640, word_ratio * stacked_word_height + 80)
    standalone_height = 128
    families = {
        'mark': (1024, 1024, mark()),
        'horizontal': (horizontal_width, 256, mark(0, 0, 256) + '\n' + wordmark(280, 72, horizontal_word_height)),
        'stacked': (stacked_width, 668, mark((stacked_width - 560) / 2, 0, 560) + '\n' + wordmark((stacked_width - word_ratio * stacked_word_height) / 2, 544, stacked_word_height)),
        'wordmark': (word_ratio * standalone_height + 48, 176, wordmark(24, 24, standalone_height)),
    }
    colors = {'': INK, '-white': '#ffffff', '-black': '#000000', '-currentcolor': 'currentColor'}
    for family, (width, height, body) in families.items():
        for suffix, color in colors.items():
            name = f'semanticdb-{family}{suffix}.svg'
            (ROOT / name).write_text(svg(f'SemanticDB {family}', width, height, body, color))

    for theme, color, bg in [('light', INK, PAPER), ('dark', PAPER, INK)]:
        (ROOT / f'semanticdb-app-icon-{theme}.svg').write_text(
            svg('SemanticDB', 1024, 1024, mark(62, 62, 900), color, bg))

    # Background keeps the favicon legible on arbitrary browser tab surfaces.
    style = f'<style>.tile{{fill:{PAPER}}}.symbol{{fill:{INK}}}@media(prefers-color-scheme:dark){{.tile{{fill:{INK}}}.symbol{{fill:{PAPER}}}}}</style>\n'
    favicon = '<rect class="tile" width="64" height="64" rx="12"/>' + f'<g class="symbol">{mark(0, 0, 64)}</g>'
    (ROOT / 'favicon.svg').write_text(svg('SemanticDB', 64, 64, favicon, extra=style))


if __name__ == '__main__':
    build()
    print(f'Built {len(list(ROOT.glob("*.svg")))} SVG assets in {ROOT}')
