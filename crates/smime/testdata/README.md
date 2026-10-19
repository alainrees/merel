Test certificates for the S/MIME tests, made with OpenSSL. Nothing here is secret: the keys
exist only for the tests, and the password of every `.p12` file is `geheim`.

- `ca.pem`: "Mailcraft Test CA", the issuer of the others.
- `jij.p12`, `jij-3des.p12`: "Jij Tester" <jij@example.org>, for e-mail; the second exported
  with Triple DES and a SHA-1 MAC, as Windows and older programs do. `jij.pem` is its certificate.
- `alain.p12`: "Alain Tester" <alain@voorbeeld.nl>, for e-mail.
- `server.p12`: a TLS server certificate, which may not sign mail.

All are valid from 19 October 2026 for a hundred years.
- `signed-openssl.eml`, `signed-ec.eml`: mail signed by `openssl smime -sign`, by "Jij Tester" (RSA) and by
  "Ec Tester" <ec@example.org> (ECDSA P-256); `signed-tampered.eml` is the first with its text changed afterwards.
- `encrypted-openssl.eml`, `encrypted-oaep.eml`: mail for "Jij Tester" encrypted by OpenSSL with AES-256-CBC, the key
  wrapped with RSAES-PKCS1-v1_5 and with RSAES-OAEP.
- `ca.der`, `jij.der`: the test CA and "Jij Tester" as DER. `sub-ca.der`: "Mailcraft Test Sub CA", an intermediate
  issued by the test CA; `sanne.der`: "Sanne Tester" <sanne@example.org>, issued by the intermediate.
  `forged.der`: a self-signed certificate claiming "Jij Tester" <jij@example.org>, which no trusted issuer made.
- `signed-chain.eml`: signed by Sanne with the intermediate enclosed; `signed-forged.eml`: signed with the forged
  certificate, From jij@example.org.
