"""MGS implementation of the observed version-1 skin-core file protocol.

No reference implementation is imported. One random key per invocation, only
sent over stdin. These envelopes authenticate a transport result, not a render.
"""
from __future__ import annotations

import json
import os
import struct

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from .provider import CosmeticError

REQUEST_MAGIC = b"SKINREQ1"
RESPONSE_MAGIC = b"SKINRES1"
MAX_FRAME = 256 * 1024


def _aad(magic: bytes, input_path: str, output_path: str) -> bytes:
    # Wire contract uses the exact CLI strings, not normalized path identities.
    return magic + b"\x01" + input_path.encode("utf-8") + output_path.encode("utf-8")


def encode_request(body: dict, input_path: str, output_path: str) -> tuple[bytes, bytes]:
    plain = json.dumps(body, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
    if len(plain) > MAX_FRAME - 37:
        raise CosmeticError("CORE_REQUEST_TOO_LARGE")
    key, nonce = os.urandom(32), os.urandom(12)
    encrypted = AESGCM(key).encrypt(nonce, plain, _aad(REQUEST_MAGIC, input_path, output_path))
    return key, REQUEST_MAGIC + b"\x01" + nonce + encrypted


def stdin_packet(key: bytes) -> bytes:
    if len(key) != 32:
        raise CosmeticError("CORE_KEY_LENGTH")
    return b"CS2SKIN1" + struct.pack("<I", len(key)) + key


def _unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate object key")
        result[key] = value
    return result


def decode_response(frame: bytes, key: bytes, input_path: str, output_path: str) -> dict:
    if len(key) != 32 or not 37 <= len(frame) <= MAX_FRAME or frame[:9] != RESPONSE_MAGIC + b"\x01":
        raise CosmeticError("CORE_RESPONSE_HEADER")
    try:
        plain = AESGCM(key).decrypt(frame[9:21], frame[21:], _aad(RESPONSE_MAGIC, input_path, output_path))
        result = json.loads(plain, object_pairs_hook=_unique_pairs,
                            parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")))
    except (InvalidTag, UnicodeError, ValueError) as exc:
        raise CosmeticError("CORE_RESPONSE_AUTHENTICATION_OR_JSON") from exc
    if not isinstance(result, dict):
        raise CosmeticError("CORE_RESPONSE_NOT_OBJECT")
    return result
