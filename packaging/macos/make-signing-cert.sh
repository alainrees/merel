#!/usr/bin/env bash
# Makes Merel's own code-signing certificate, once, on a Mac. Free: no Apple account. Every
# release signed with it has the same identity, so "Sta altijd toe" in the keychain keeps
# working after an update. Run from the repository root:
#   packaging/macos/make-signing-cert.sh
# It leaves merel-codesign.p12 in your home folder and puts each GitHub secret on the
# clipboard in turn. Keep the .p12 and its password in your password manager: GitHub never
# shows a secret again, and a new certificate means the keychain asks once more.
set -euo pipefail
openssl=/usr/bin/openssl # macOS's own, whose .p12 files `security import` reads
out="$HOME/merel-codesign.p12"
[ -e "$out" ] && { echo "$out bestaat al; gebruik die of verwijder hem eerst." >&2; exit 1; }
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cat > "$work/cert.cnf" <<'CNF'
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = Merel
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
CNF
"$openssl" req -x509 -newkey rsa:3072 -nodes -days 7300 -config "$work/cert.cnf" \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
password="$("$openssl" rand -hex 24)"
"$openssl" pkcs12 -export -name Merel -inkey "$work/key.pem" -in "$work/cert.pem" \
  -out "$out" -passout "pass:$password"
chmod 600 "$out"

github=https://github.com/alainrees/merel/settings/secrets/actions/new
base64 -i "$out" | tr -d '\n' | pbcopy
echo "1. Het certificaat staat op het klembord. Maak in GitHub een secret MACOS_CERT_P12 en plak het:"
echo "   $github"
read -r -p "   Druk op Enter als dat gedaan is. "
printf '%s' "$password" | pbcopy
echo "2. Het wachtwoord staat op het klembord. Maak een secret MACOS_CERT_PASSWORD en plak het."
read -r -p "   Druk op Enter als dat gedaan is. "
printf '' | pbcopy
echo "3. Bewaar $out en dit wachtwoord in je wachtwoordbeheerder:"
echo "   $password"
