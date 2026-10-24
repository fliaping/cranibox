"""Verify probe output independently of Apple: python3 -I verify.py '<json line>'."""
import json
import sys

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec

result = json.loads(sys.argv[1])
public = ec.EllipticCurvePublicKey.from_encoded_point(
    ec.SECP256R1(), bytes.fromhex(result["public_x963"])
)
public.verify(
    bytes.fromhex(result["signature_der"]),
    bytes.fromhex(result["message"]),
    ec.ECDSA(hashes.SHA256()),
)
print("signature valid")
