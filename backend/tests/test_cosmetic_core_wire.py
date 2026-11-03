import json
import pytest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from app.cosmetic_studio.core_wire import (
    REQUEST_MAGIC, RESPONSE_MAGIC, decode_response, encode_request, stdin_packet,
)
from app.cosmetic_studio.provider import CosmeticError


def test_encode_decode_round_trip_and_exact_path_aad():
    key, frame = encode_request({"schema_version": 1, "x": "中文"}, r"D:\in.dem", r"D:\out.dem")
    assert stdin_packet(key).startswith(b"CS2SKIN1")
    body = json.dumps({"schema_version": 1, "ok": True}, separators=(",", ":")).encode()
    nonce = b"N" * 12
    encrypted = AESGCM(key).encrypt(nonce, body, RESPONSE_MAGIC + b"\x01D:\\in.demD:\\out.dem")
    response = RESPONSE_MAGIC + b"\x01" + nonce + encrypted
    assert decode_response(response, key, r"D:\in.dem", r"D:\out.dem")["ok"] is True
    with pytest.raises(CosmeticError):
        decode_response(response, key, r"D:\other.dem", r"D:\out.dem")


@pytest.mark.parametrize("mutator", [lambda b: b[:30] + bytes([b[30] ^ 1]) + b[31:], lambda b: b[:-1] + bytes([b[-1] ^ 1])])
def test_tampered_response_is_rejected(mutator):
    key, _ = encode_request({"schema_version": 1}, "D:\\in.dem", "D:\\out.dem")
    nonce = b"Q" * 12
    enc = AESGCM(key).encrypt(nonce, b"{}", RESPONSE_MAGIC + b"\x01D:\\in.demD:\\out.dem")
    frame = RESPONSE_MAGIC + b"\x01" + nonce + enc
    with pytest.raises(CosmeticError):
        decode_response(mutator(frame), key, "D:\\in.dem", "D:\\out.dem")


def test_bad_header_duplicate_key_and_nonfinite_json_rejected():
    key, _ = encode_request({"schema_version": 1}, "D:\\in.dem", "D:\\out.dem")
    for payload in (b'{"x":1,"x":2}', b'{"x":NaN}', b'[]'):
        nonce = b"R" * 12
        enc = AESGCM(key).encrypt(nonce, payload, RESPONSE_MAGIC + b"\x01D:\\in.demD:\\out.dem")
        frame = RESPONSE_MAGIC + b"\x01" + nonce + enc
        with pytest.raises(CosmeticError):
            decode_response(frame, key, "D:\\in.dem", "D:\\out.dem")
    with pytest.raises(CosmeticError):
        decode_response(b"BAD" * 20, key, "D:\\in.dem", "D:\\out.dem")


def test_request_rejects_nan_and_oversized_payload():
    with pytest.raises(ValueError):
        encode_request({"value": float("nan")}, "D:\\in.dem", "D:\\out.dem")
    with pytest.raises(CosmeticError):
        encode_request({"x": "a" * (256 * 1024)}, "D:\\in.dem", "D:\\out.dem")

