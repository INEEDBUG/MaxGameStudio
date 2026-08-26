"""Domain models for native VALORANT crosshair profile codes.

The game exports a compact semicolon-delimited profile rather than JSON.  The
models in this module deliberately keep the ordered field list so importing a
profile and exporting it again does not discard fields introduced by a newer
game version.
"""

from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator


CrosshairSectionName = Literal["P", "A", "S"]


class CrosshairField(BaseModel):
    """One key/value pair from a profile code.

    ``value`` is the value emitted by the serializer.  ``raw_value`` is kept
    separately so callers can inspect what was imported; unknown fields retain
    that value byte-for-byte apart from harmless surrounding whitespace.
    """

    model_config = ConfigDict(validate_assignment=True)

    key: str = Field(min_length=1, max_length=64)
    value: str = Field(min_length=1, max_length=1024)
    raw_value: str | None = Field(default=None, max_length=1024)
    known: bool = False
    section: str = Field(default="start", min_length=1, max_length=8)

    @field_validator("key", "section")
    @classmethod
    def no_semicolon_or_whitespace(cls, value: str) -> str:
        if ";" in value or any(char.isspace() for char in value):
            raise ValueError("crosshair field names and sections cannot contain whitespace or ';'")
        return value

    @field_validator("value")
    @classmethod
    def value_is_single_token(cls, value: str) -> str:
        if ";" in value or any(char in value for char in "\r\n"):
            raise ValueError("crosshair field values cannot contain ';' or line breaks")
        return value

    @field_validator("raw_value")
    @classmethod
    def raw_value_is_single_token(cls, value: str | None) -> str | None:
        if value is not None and (";" in value or any(char in value for char in "\r\n")):
            raise ValueError("raw crosshair values cannot contain ';' or line breaks")
        return value

    @model_validator(mode="after")
    def default_raw_value(self) -> "CrosshairField":
        if self.raw_value is None:
            self.raw_value = self.value
        return self

    @property
    def normalized_value(self) -> str:
        """Alias useful to code that distinguishes raw and canonical values."""

        return self.value

    @property
    def is_known(self) -> bool:
        return self.known


class CrosshairSection(BaseModel):
    """An ordered ``P``, ``A`` or ``S`` section."""

    model_config = ConfigDict(validate_assignment=True)

    name: CrosshairSectionName
    fields: list[CrosshairField] = Field(default_factory=list, max_length=256)

    @model_validator(mode="after")
    def fields_belong_to_section(self) -> "CrosshairSection":
        for field in self.fields:
            if field.section == "start":
                field.section = self.name
        return self

    def get(self, key: str, default: str | None = None) -> str | None:
        """Return the last value for ``key`` in this section."""

        for field in reversed(self.fields):
            if field.key == key:
                return field.value
        return default

    def get_field(self, key: str) -> CrosshairField | None:
        for field in reversed(self.fields):
            if field.key == key:
                return field
        return None

    @property
    def values(self) -> dict[str, str]:
        """Mapping view; duplicate keys resolve to the last occurrence."""

        return {field.key: field.value for field in self.fields}


class ValorantCrosshairAST(BaseModel):
    """Ordered abstract syntax tree for a native VALORANT profile code."""

    model_config = ConfigDict(validate_assignment=True)

    version: Literal[0] = 0
    start_fields: list[CrosshairField] = Field(default_factory=list, max_length=128)
    sections: list[CrosshairSection] = Field(default_factory=list, max_length=3)

    @model_validator(mode="after")
    def validate_sections(self) -> "ValorantCrosshairAST":
        names = [section.name for section in self.sections]
        if names.count("P") != 1:
            raise ValueError("a VALORANT crosshair AST requires exactly one P section")
        if len(names) != len(set(names)):
            raise ValueError("VALORANT crosshair sections cannot be repeated")
        for field in self.start_fields:
            if field.section != "start":
                field.section = "start"
        return self

    @property
    def prefix_fields(self) -> list[CrosshairField]:
        """Compatibility alias for the fields before the first profile block."""

        return self.start_fields

    @property
    def profile_sections(self) -> list[CrosshairSection]:
        return self.sections

    @property
    def start_values(self) -> dict[str, str]:
        return {field.key: field.value for field in self.start_fields}

    @property
    def profiles(self) -> dict[str, CrosshairSection]:
        """Mapping view for callers that prefer named profile access."""

        return {section.name: section for section in self.sections}

    @classmethod
    def from_code(cls, code: str, **kwargs: Any) -> "ValorantCrosshairAST":
        from .codec import parse_crosshair_code

        return parse_crosshair_code(code, **kwargs)

    def section(self, name: CrosshairSectionName) -> CrosshairSection | None:
        return next((section for section in self.sections if section.name == name), None)

    def require_section(self, name: CrosshairSectionName) -> CrosshairSection:
        section = self.section(name)
        if section is None:
            raise KeyError(name)
        return section

    def get(self, section: str, key: str, default: str | None = None) -> str | None:
        if section in {"", "0", "start"}:
            for field in reversed(self.start_fields):
                if field.key == key:
                    return field.value
            return default
        profile = self.section(section)  # type: ignore[arg-type]
        return default if profile is None else profile.get(key, default)

    def values(self, section: str = "P") -> dict[str, str]:
        if section in {"", "0", "start"}:
            return {field.key: field.value for field in self.start_fields}
        return self.require_section(section).values  # type: ignore[arg-type]

    def set_field(self, section: str, key: str, value: Any) -> CrosshairField:
        """Set or append a field, applying the same normalization as parsing."""

        from .codec import normalize_field

        target: list[CrosshairField]
        section_name = "start" if section in {"", "0", "start"} else section
        if section_name == "start":
            target = self.start_fields
        else:
            if section_name not in {"P", "A", "S"}:
                raise ValueError(f"unsupported VALORANT section: {section}")
            profile = self.section(section_name)  # type: ignore[arg-type]
            if profile is None:
                profile = CrosshairSection(name=section_name)  # type: ignore[arg-type]
                self.sections.append(profile)
            target = profile.fields

        normalized, known = normalize_field(section_name, key, str(value), strict=True)
        for field in target:
            if field.key == key:
                field.value = normalized
                field.raw_value = str(value)
                field.known = known
                return field
        field = CrosshairField(
            key=key,
            value=normalized,
            raw_value=str(value),
            known=known,
            section=section_name,
        )
        target.append(field)
        return field

    def remove_field(self, section: str, key: str) -> bool:
        target = self.start_fields if section in {"", "0", "start"} else self.require_section(section).fields  # type: ignore[arg-type]
        before = len(target)
        target[:] = [field for field in target if field.key != key]
        return len(target) != before

    def to_code(self, *, preserve_unknown: bool = True, canonical: bool = False) -> str:
        from .codec import serialize_crosshair_code

        return serialize_crosshair_code(self, preserve_unknown=preserve_unknown, canonical=canonical)

    serialize = to_code


def _normalize_tags(values: list[str]) -> list[str]:
    output: list[str] = []
    seen: set[str] = set()
    for raw in values:
        tag = str(raw).strip()
        if not tag:
            continue
        folded = tag.casefold()
        if folded not in seen:
            output.append(tag)
            seen.add(folded)
    return output


def _validate_code(value: str) -> str:
    from .codec import parse_crosshair_code, serialize_crosshair_code

    ast = parse_crosshair_code(value)
    return serialize_crosshair_code(ast)


class CrosshairPreset(BaseModel):
    """A named, local preset backed by a native profile code."""

    model_config = ConfigDict(validate_assignment=True)

    id: int | None = Field(default=None, ge=1)
    name: str = Field(min_length=1, max_length=200)
    code: str = Field(min_length=1, max_length=16_384)
    description: str = Field(default="", max_length=2_000)
    tags: list[str] = Field(default_factory=list, max_length=32)
    source: str | None = Field(default=None, max_length=500)
    created_at: str | None = Field(default=None, max_length=64)
    updated_at: str | None = Field(default=None, max_length=64)

    @field_validator("name", "description")
    @classmethod
    def trim_text(cls, value: str) -> str:
        return value.strip()

    @field_validator("code")
    @classmethod
    def normalize_native_code(cls, value: str) -> str:
        return _validate_code(value)

    @field_validator("tags")
    @classmethod
    def normalize_preset_tags(cls, values: list[str]) -> list[str]:
        return _normalize_tags(values)

    @model_validator(mode="after")
    def require_name(self) -> "CrosshairPreset":
        if not self.name:
            raise ValueError("preset name must not be blank")
        return self

    @property
    def ast(self) -> ValorantCrosshairAST:
        from .codec import parse_crosshair_code

        return parse_crosshair_code(self.code)


class CrosshairPresetCreate(BaseModel):
    name: str = Field(min_length=1, max_length=200)
    code: str = Field(min_length=1, max_length=16_384)
    description: str = Field(default="", max_length=2_000)
    tags: list[str] = Field(default_factory=list, max_length=32)
    source: str | None = Field(default=None, max_length=500)

    @field_validator("name", "description")
    @classmethod
    def trim_create_text(cls, value: str) -> str:
        return value.strip()

    @field_validator("code")
    @classmethod
    def normalize_create_code(cls, value: str) -> str:
        return _validate_code(value)

    @field_validator("tags")
    @classmethod
    def normalize_create_tags(cls, values: list[str]) -> list[str]:
        return _normalize_tags(values)

    @model_validator(mode="after")
    def require_create_name(self) -> "CrosshairPresetCreate":
        if not self.name:
            raise ValueError("preset name must not be blank")
        return self


class CrosshairPresetPatch(BaseModel):
    name: str | None = Field(default=None, min_length=1, max_length=200)
    code: str | None = Field(default=None, min_length=1, max_length=16_384)
    description: str | None = Field(default=None, max_length=2_000)
    tags: list[str] | None = Field(default=None, max_length=32)
    source: str | None = Field(default=None, max_length=500)

    @field_validator("name", "description")
    @classmethod
    def trim_patch_text(cls, value: str | None) -> str | None:
        return None if value is None else value.strip()

    @field_validator("code")
    @classmethod
    def normalize_patch_code(cls, value: str | None) -> str | None:
        return None if value is None else _validate_code(value)

    @field_validator("tags")
    @classmethod
    def normalize_patch_tags(cls, values: list[str] | None) -> list[str] | None:
        return None if values is None else _normalize_tags(values)


# Friendly aliases used by callers that prefer an explicit Valorant prefix.
ValorantCrosshairField = CrosshairField
ValorantCrosshairSection = CrosshairSection
ValorantCrosshairCode = ValorantCrosshairAST
ValorantCrosshairPreset = CrosshairPreset


__all__ = [
    "CrosshairSectionName",
    "CrosshairField",
    "CrosshairSection",
    "ValorantCrosshairAST",
    "CrosshairPreset",
    "CrosshairPresetCreate",
    "CrosshairPresetPatch",
    "ValorantCrosshairField",
    "ValorantCrosshairSection",
    "ValorantCrosshairCode",
    "ValorantCrosshairPreset",
]
