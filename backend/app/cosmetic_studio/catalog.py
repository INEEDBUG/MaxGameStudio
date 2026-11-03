"""Finite firearm finishes plus separately qualified knife/glove transformations.

This contains identifiers, not bundled Valve assets. Runtime/visual qualification
is independent of whether a finish exists in the game schema.
"""
import hashlib
import json
from pathlib import Path


def _gun_catalog():
    # Local, pinned metadata only. A website listing cannot authorize arbitrary
    # caller paint IDs, new replacement models, or manufacture item ownership.
    raw = Path(__file__).with_name('gun_finish_catalog.json').read_bytes()
    if hashlib.sha256(raw).hexdigest() != '2bfa89da9d478c29789957ffefbbdf7db30b41a6d68fa2c2e222914890517d6d':
        raise ValueError('Firearm catalog integrity check failed')
    data = json.loads(raw)
    if (data.get('schema_version') != 1
            or data.get('commit') != 'e2644abaf1bb26b871d0e90e7b44b7116feef486'
            or data.get('source_sha256') != '001439bb7b2943ea7ed659d74c7c8179378edd512ccd2cc37ce15e9681df1b3f'):
        raise ValueError('Firearm catalog provenance is invalid')
    return data


_GUN_CATALOG = _gun_catalog()
GUN_FINISH_METADATA = _GUN_CATALOG['items']

WEAPON_NAMES = {
    9: ('awp', 'weapon_awp', 'c_weaponawp'),
    7: ('ak47', 'weapon_ak47', 'c_weaponak47', 'ak-47'),
    16: ('m4a1', 'weapon_m4a1', 'm4a4'),
    60: ('m4a1_silencer', 'weapon_m4a1_silencer', 'm4a1-s'),
    1: ('deagle', 'weapon_deagle', 'c_weapondeagle', 'desert eagle'),
    61: ('usp_silencer', 'weapon_usp_silencer', 'usp-s'),
    4: ('glock', 'weapon_glock', 'c_weaponglock', 'glock-18'),
    507: ('karambit', 'weapon_knife_karambit'),
    515: ('butterfly', 'butterfly knife', 'knife_butterfly', 'weapon_knife_butterfly'),
}

for _definition, _weapon in _GUN_CATALOG['weapons'].items():
    WEAPON_NAMES.setdefault(int(_definition), (_weapon.removeprefix('weapon_'), _weapon))

_DISPLAY_ALIASES = {2: ('dual berettas',), 3: ('five-seven',), 13: ('galil ar',),
                    17: ('mac-10',), 19: ('p90',), 23: ('mp5-sd',), 24: ('ump-45',),
                    26: ('pp-bizon', 'pp bizon'), 27: ('mag-7',), 29: ('sawed-off',),
                    30: ('tec-9',), 32: ('p2000',), 38: ('scar-20',),
                    39: ('sg 553', 'sg553', 'sg-553'), 40: ('ssg 08', 'ssg-08'),
                    63: ('cz75-auto',), 64: ('r8 revolver', 'r8')}
for _definition, _aliases in _DISPLAY_ALIASES.items():
    WEAPON_NAMES[_definition] = (*WEAPON_NAMES[_definition], *_aliases)

# CS2's live entity class is shared by these model variants. These aliases
# must NOT classify parser inventory: the exact definition is independently
# verified in the immutable playback copy across the complete recording span.
NATIVE_WEAPON_NAMES = {**WEAPON_NAMES,
    60: (*WEAPON_NAMES[60], 'weapon_m4a1'),
    61: (*WEAPON_NAMES[61], 'weapon_hkp2000'),
    507: (*WEAPON_NAMES[507], 'weapon_knife'),
    515: (*WEAPON_NAMES[515], 'weapon_knife', 'c_weaponknife')}


def native_weapon_matches(value, definition: int) -> bool:
    text = str(value or '').strip().lower().rsplit('/', 1)[-1]
    return text in NATIVE_WEAPON_NAMES.get(definition, ())

# id -> definition, paint, label. Adding arbitrary paints or replacement models
# is deliberately not an HTTP capability.
PRESET_DATA = {
    'awp-dragon-lore': (9, 344, 'AWP · 巨龙传说'),
    'awp-asiimov': (9, 279, 'AWP · 二西莫夫'),
    'awp-medusa': (9, 446, 'AWP · 美杜莎'),
    'ak47-fire-serpent': (7, 180, 'AK-47 · 火蛇'),
    'ak47-wild-lotus': (7, 724, 'AK-47 · 野荷'),
    'ak47-gold-arabesque': (7, 921, 'AK-47 · 黄金藤蔓'),
    'm4a4-howl': (16, 309, 'M4A4 · 咆哮'),
    'm4a1s-hot-rod': (60, 445, 'M4A1-S · 赤红新星'),
    'm4a1s-blue-phosphor': (60, 1017, 'M4A1-S · 澜磷'),
    'deagle-blaze': (1, 37, '沙漠之鹰 · 炽烈之炎'),
    'usp-kill-confirmed': (61, 504, 'USP-S · 枪响人亡'),
    'glock-fade': (4, 38, '格洛克 · 渐变之色'),
    # Same-model 507/38 independently parsed and visually qualified in a
    # dedicated current-build native Demo. Knife model replacement is NOT
    # supported merely because the live engine shares weapon_knife.
    'karambit-fade': (507, 38, '爪子刀 · 渐变之色'),
    # Explicit 507 -> 515 qualification: immutable file parser comparison,
    # native offline preview, OBS recording and LiteCut export, 2026-10-06
    # job a3dd6d7b8cb54cb4ad137b9258adca87. Not a general knife-model API.
    'karambit-to-butterfly-fade': (515, 38, '爪子刀外观 → 蝴蝶刀 · 渐变之色'),
}

# Keep persisted legacy preset IDs, with one menu entry per gun/paint pair.
_existing_pairs = {(definition, paint) for definition, paint, _ in PRESET_DATA.values()}
for _key, _metadata in GUN_FINISH_METADATA.items():
    _definition, _paint = map(int, _key.split(':'))
    if (_definition, _paint) not in _existing_pairs:
        PRESET_DATA[f'gun-{_definition}-{_paint}'] = (_definition, _paint, _metadata['name'])

WEAR_LIMITS = {tuple(map(int, key.split(':'))): (value['wear_min'], value['wear_max'])
               for key, value in GUN_FINISH_METADATA.items()}
WEAR_LIMITS.update({(507, 38): (0.0, 0.08), (515, 38): (0.0, 0.08)})

_CROSS_MODEL_SOURCES = {'karambit-to-butterfly-fade': 507}


def source_weapon_id(preset_id: str) -> int:
    """Return the observed source definition; replacement targets are never caller-selected."""
    if preset_id not in PRESET_DATA:
        raise ValueError('Unsupported cosmetic preset')
    return _CROSS_MODEL_SOURCES.get(preset_id, PRESET_DATA[preset_id][0])

# Accepted only after native + OBS + LiteCut combination qualification,
# 2026-10-06 job e8bd8660d6a4426bba423afc2a02138f. Exact native build pins
# are independently enforced; this is not a generic glove replacement API.
GLOVE_PRESET_DATA = {'specialist-10061': '专家手套 · 10061（离线实验）'}
GLOVE_SUPPORTED_WEAPONS = frozenset({507})


def weapon_matches(value, definition: int) -> bool:
    text = str(value or '').strip().lower()
    return text in WEAPON_NAMES.get(definition, ()) or text.rsplit('/', 1)[-1] in WEAPON_NAMES.get(definition, ())


def weapon_definition(value) -> int | None:
    return next((definition for definition in WEAPON_NAMES if weapon_matches(value, definition)), None)
