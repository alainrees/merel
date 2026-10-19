# Accounts toevoegen

## IMAP/SMTP (wachtwoord)

Bestand › Accounts › Account toevoegen. Vul naam, e-mailadres en wachtwoord in; de knop
"Servers invullen op basis van het adres" raadt `imap.<domein>` en `smtp.<domein>`. Je
wachtwoord gaat in de sleutelhanger van je computer, niet in een bestand.

### Agenda en adresboek bij een IMAP-account (CalDAV/CardDAV)

De meeste aanbieders die IMAP spreken, bewaren je agenda en adresboek op een CalDAV- en
CardDAV-server. Vul onder "Agenda en adresboek (CalDAV/CardDAV), optioneel" het https-adres
van die server in; Mailcraft zoekt daar zelf de agenda's, takenlijsten en adresboeken op, met
dezelfde gebruikersnaam en hetzelfde wachtwoord als voor de mail. Laat het veld Adresboek leeg
als het op dezelfde server staat als de agenda. Laat beide leeg voor een account met alleen mail.

| Aanbieder | Agenda | Adresboek |
|---|---|---|
| Fastmail | `https://caldav.fastmail.com/` | `https://carddav.fastmail.com/` |
| iCloud | `https://caldav.icloud.com/` | `https://contacts.icloud.com/` |
| Nextcloud | `https://cloud.voorbeeld.nl/remote.php/dav/` | (leeg) |
| Posteo | `https://posteo.de:8443/` | (leeg) |
| mailbox.org | `https://dav.mailbox.org/` | (leeg) |

De knop "Servers invullen op basis van het adres" vult Fastmail, iCloud, Posteo en mailbox.org
zelf in. iCloud en Fastmail met tweestapsverificatie willen een app-specifiek wachtwoord; dat
geldt dan voor mail en agenda samen. Herhalende afspraken worden getoond zoals de server ze
uitwerkt, maar kun je in Mailcraft nog niet bewerken.

## Gmail

Google staat geen gewoon wachtwoord toe; je logt in via de browser (OAuth 2.0). Daarvoor moet
de app één keer bij Google geregistreerd zijn. Dat is gratis en duurt een paar minuten:

1. Ga naar <https://console.cloud.google.com/> en maak een project aan (bijvoorbeeld
   "Mailcraft"). Zet onder **APIs & Services › Library** de *Google Calendar API*, de
   *People API* en de *Tasks API* aan; zonder die drie komt alleen de mail mee.
2. **APIs & Services › OAuth consent screen**: kies *External*, vul een appnaam en je
   e-mailadres in, en voeg jezelf toe bij *Test users*. Publiceren is niet nodig.
3. **APIs & Services › Credentials › Create credentials › OAuth client ID**: type
   *Desktop app*. Je krijgt een **Client ID** (eindigt op `.apps.googleusercontent.com`) en een
   **Client secret**.
4. In Mailcraft: Bestand › Accounts › soort **Gmail**, vul naam, e-mailadres, client-ID en
   client-secret in en klik op **Inloggen bij Google**. Je browser opent; na het inloggen zegt
   de pagina "Gelukt" en verschijnt het account in Mailcraft.

Mailcraft vraagt de machtigingen `https://mail.google.com/` (IMAP en SMTP) en die voor
Google Agenda, Contacten en Taken. Het vernieuwingstoken en het client-secret gaan in de
sleutelhanger.

Met die machtigingen komen de agenda, de contacten en de taken van het Google-account mee, net
als bij Exchange: de agenda verschijnt naast "Mijn agenda", contacten en taken staan tussen de
lokale, en wijzigingen gaan meteen naar Google. Google Taken kent geen prioriteit; die blijft
alleen in Mailcraft staan. Een Gmail-account dat is toegevoegd toen Mailcraft alleen om mail
vroeg, laat alleen mail zien: klik bij dat account op **Opnieuw aanmelden** (Bestand ›
Accounts) om de extra toestemming te geven.

## Microsoft 365 / Outlook.com

Ook Microsoft werkt via de browser. Mailcraft praat met Microsoft Graph: mail, verzenden,
agenda, contacten en taken komen alle via één aanmelding mee, zonder IMAP of SMTP. Registreer
de app één keer in Entra:

1. Ga naar <https://entra.microsoft.com/> › **App registrations › New registration**.
2. Naam bijvoorbeeld "Mailcraft"; bij *Supported account types* kies "Accounts in any
   organizational directory and personal Microsoft accounts"; bij *Redirect URI* kies
   **Public client/native (mobile & desktop)** met waarde `http://localhost`.
3. Onder **API permissions** voeg je toe: *Microsoft Graph* › Delegated › `Mail.ReadWrite`,
   `Mail.Send`, `Calendars.ReadWrite`, `Contacts.ReadWrite`, `Tasks.ReadWrite` en
   `offline_access`.
4. Onder **Authentication** zet je *Allow public client flows* op Yes.
5. In Mailcraft: Bestand › Accounts › soort **Microsoft 365**, vul naam, e-mailadres en de
   **Application (client) ID** in, zonder client-secret, en klik op **Inloggen bij Microsoft**.

Let op: waar een zakelijke tenant dat eist, moet een beheerder de app goedkeuren. Een
Microsoft 365-account dat is toegevoegd toen Mailcraft nog IMAP gebruikte, blijft mail doen
via IMAP; voeg in Entra de Graph-machtigingen toe en klik bij het account op **Opnieuw
aanmelden** (Bestand › Accounts) om over te gaan op Graph, met agenda, contacten en taken.
Exchange Online stopt in oktober 2026 met EWS; daarom gaat Microsoft 365 niet via de
Exchange-soort maar via Graph.

## Exchange Server (on-premises)

Voor een eigen Exchange Server (2013 of nieuwer) gebruikt Mailcraft Exchange Web Services (EWS)
over HTTPS; lezen, verplaatsen én verzenden gaan via dezelfde server, er is geen SMTP nodig.

Bestand › Accounts › soort **Exchange**: vul naam, e-mailadres, gebruikersnaam (`DOMEIN\naam` of
je e-mailadres), wachtwoord en de servernaam in (bijvoorbeeld `mail.bedrijf.nl`; Mailcraft
praat met `https://mail.bedrijf.nl/EWS/Exchange.asmx`). Het certificaat van de server moet
geldig zijn; een zelfondertekend certificaat wordt geweigerd.

Bij een Exchange-account komen ook de agenda, de contacten en de taken mee: de agenda van het
account verschijnt naast "Mijn agenda" (afspraken van 90 dagen terug tot 400 dagen vooruit),
contacten en taken staan tussen de lokale met de accountnaam erbij. Wat je in Mailcraft
wijzigt, gaat meteen naar de server; bij een nieuwe afspraak, contactpersoon of taak kies je
of die op deze computer of in het account komt. **Vernieuwen** in het lint haalt alles opnieuw
op. Zonder verbinding zie je alleen de lokale items.

Let op: Mailcraft meldt zich aan met *Basic authentication* over TLS. Veel servers staan voor
EWS alleen NTLM of Kerberos toe; Mailcraft zegt dat dan en de beheerder kan Basic voor EWS
inschakelen. Voor Exchange Online (Microsoft 365) gebruik je de soort **Microsoft 365**.

## Handtekening

Bestand › Accounts, bij het account **Handtekening instellen** (later **Handtekening bewerken**).
De tekst komt onder elk nieuw bericht, antwoord en doorgestuurd bericht van dat account, boven
het aangehaalde bericht, met de gebruikelijke `-- ` ervoor zodat andere mailprogramma's hem
herkennen. Elk account heeft zijn eigen handtekening; leeg betekent geen.

## Account verwijderen

Bestand › Accounts, bij het account de knop **Account verwijderen**, en dan **Ja, verwijderen**.
Het account, het wachtwoord of de aanmelding in de sleutelhanger en de versleutelde offline
kopie verdwijnen van deze computer; de afspraken, contactpersonen en taken die bij het account
hoorden ook. Je mail en je agenda bij de aanbieder blijven staan. Afspraken, contactpersonen en
taken die op deze computer staan ("Mijn agenda") blijven. Zonder accounts zie je weer de
voorbeeldmail.

## Hoe het aanmelden werkt

Mailcraft opent je eigen browser en luistert ondertussen op een willekeurige poort op
`127.0.0.1`, alleen voor deze ene aanmelding. De app ziet je wachtwoord nooit. Het verzoek
gebruikt PKCE en een state-waarde, zodat een andere app op je computer de code niet kan
onderscheppen of onderschuiven. Alleen het vernieuwingstoken wordt bewaard, in de
sleutelhanger; toegangstokens leven in het geheugen en verlopen vanzelf.
