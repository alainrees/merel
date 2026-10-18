# Accounts toevoegen

## IMAP/SMTP (wachtwoord)

Bestand › Accounts › Account toevoegen. Vul naam, e-mailadres en wachtwoord in; de knop
"Servers invullen op basis van het adres" raadt `imap.<domein>` en `smtp.<domein>`. Je
wachtwoord gaat in de sleutelhanger van je computer, niet in een bestand.

## Gmail

Google staat geen gewoon wachtwoord toe; je logt in via de browser (OAuth 2.0). Daarvoor moet
de app één keer bij Google geregistreerd zijn. Dat is gratis en duurt een paar minuten:

1. Ga naar <https://console.cloud.google.com/> en maak een project aan (bijvoorbeeld
   "Mailcraft").
2. **APIs & Services › OAuth consent screen**: kies *External*, vul een appnaam en je
   e-mailadres in, en voeg jezelf toe bij *Test users*. Publiceren is niet nodig.
3. **APIs & Services › Credentials › Create credentials › OAuth client ID**: type
   *Desktop app*. Je krijgt een **Client ID** (eindigt op `.apps.googleusercontent.com`) en een
   **Client secret**.
4. In Mailcraft: Bestand › Accounts › soort **Gmail**, vul naam, e-mailadres, client-ID en
   client-secret in en klik op **Inloggen bij Google**. Je browser opent; na het inloggen zegt
   de pagina "Gelukt" en verschijnt het account in Mailcraft.

Mailcraft vraagt alleen de machtiging `https://mail.google.com/` (IMAP en SMTP). Het
vernieuwingstoken en het client-secret gaan in de sleutelhanger.

## Microsoft 365 / Outlook.com

Ook Microsoft werkt via de browser. Registreer de app één keer in Entra:

1. Ga naar <https://entra.microsoft.com/> › **App registrations › New registration**.
2. Naam bijvoorbeeld "Mailcraft"; bij *Supported account types* kies "Accounts in any
   organizational directory and personal Microsoft accounts"; bij *Redirect URI* kies
   **Public client/native (mobile & desktop)** met waarde `http://localhost`.
3. Onder **API permissions** voeg je toe (Microsoft Graph is niet nodig): *Office 365
   Exchange Online* › Delegated › `IMAP.AccessAsUser.All` en `SMTP.Send`, plus
   `offline_access`.
4. Onder **Authentication** zet je *Allow public client flows* op Yes.
5. In Mailcraft: Bestand › Accounts › soort **Microsoft 365**, vul naam, e-mailadres en de
   **Application (client) ID** in, zonder client-secret, en klik op **Inloggen bij Microsoft**.

Let op: een beheerder van een zakelijke tenant moet IMAP/SMTP met OAuth toestaan en, waar de
tenant dat eist, de app goedkeuren.

## Exchange Server (on-premises)

Voor een eigen Exchange Server (2013 of nieuwer) gebruikt Mailcraft Exchange Web Services (EWS)
over HTTPS; lezen, verplaatsen én verzenden gaan via dezelfde server, er is geen SMTP nodig.

Bestand › Accounts › soort **Exchange**: vul naam, e-mailadres, gebruikersnaam (`DOMEIN\naam` of
je e-mailadres), wachtwoord en de servernaam in (bijvoorbeeld `mail.bedrijf.nl`; Mailcraft
praat met `https://mail.bedrijf.nl/EWS/Exchange.asmx`). Het certificaat van de server moet
geldig zijn; een zelfondertekend certificaat wordt geweigerd.

Let op: Mailcraft meldt zich aan met *Basic authentication* over TLS. Veel servers staan voor
EWS alleen NTLM of Kerberos toe; Mailcraft zegt dat dan en de beheerder kan Basic voor EWS
inschakelen. Voor Exchange Online (Microsoft 365) gebruik je de soort **Microsoft 365**.

## Hoe het aanmelden werkt

Mailcraft opent je eigen browser en luistert ondertussen op een willekeurige poort op
`127.0.0.1`, alleen voor deze ene aanmelding. De app ziet je wachtwoord nooit. Het verzoek
gebruikt PKCE en een state-waarde, zodat een andere app op je computer de code niet kan
onderscheppen of onderschuiven. Alleen het vernieuwingstoken wordt bewaard, in de
sleutelhanger; toegangstokens leven in het geheugen en verlopen vanzelf.
