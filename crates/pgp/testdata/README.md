Test keys and mail for the OpenPGP tests, made with GnuPG 2.4. Nothing here is secret: the keys
exist only for the tests, and the passphrase of every secret key is `geheim`.

- `alain-secret.asc`, `alain-public.asc`: "Alain Tester" <alain@voorbeeld.nl>, Ed25519 with a Curve25519
  encryption subkey, as `gpg --quick-gen-key` makes them.
- `jij-secret.asc`, `jij-public.asc`: "Jij Tester" <jij@example.org>, the same kind.
- `rsa-secret.asc`, `rsa-public.asc`: "Rsa Tester" <rsa@example.org>, RSA 3072 with an RSA encryption subkey.
- `signed-gpg.eml`: PGP/MIME signed by Jij (`gpg --detach-sign`); `signed-gpg-tampered.eml` is it with its
  text changed afterwards; `signed-rsa.eml` is signed by the RSA key.
- `encrypted-gpg.eml`: PGP/MIME, signed by Jij and encrypted for Alain and Jij (`gpg --sign --encrypt`), which
  GnuPG 2.4 writes with its own OCB packet; `encrypted-rsa.eml`: encrypted for the RSA key only.
