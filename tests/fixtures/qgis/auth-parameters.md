# Authenticatieparameters in QGIS-verbindingen

De broncontrole van 3 november 2026 dekt de drie letterlijke Ordnance Survey-reproducties, de zes GeoServer- en HERE-reproducties en de gangbare WMS-/XYZ-parameters uit de [Leaflet Providers-broncode](https://github.com/leaflet-extras/leaflet-providers/blob/master/leaflet-providers.js). De opnieuw gelezen bestandsblob is `6574503328dce05991f24db86bcd40d76dbab6bc`. Daarnaast zijn de onderstaande primaire documentatie en publieke tegenvoorbeelden gecontroleerd.

De tests maken geen netwerkverbinding. De URL's zijn invoer voor de authenticatiecontrole; deze tests bewijzen niet dat ieder eindpunt een WMS of een nog werkende tegeldienst aanbiedt. Alle waarden zijn verzonnen. De negen reviewgevallen en de 58 auditgevallen gaan door parser, echte egui-voorvertoning en echte opslag in een tijdelijk `servers.json`. Van de auditgevallen moeten er 46 worden geblokkeerd en blijven er twaalf beschikbaar.

De onderstaande keuzes vormen het testcontract voor deze reparatie. De CI op de uiteindelijke reparatiehead moet bevestigen dat de code eraan voldoet.

## Herkenning

Namen worden net als voorheen percentgedecodeerd, hoofdletterongevoelig gemaakt en zonder streepjes of underscores vergeleken. De oorspronkelijke querywaarden worden niet veranderd.

| Naam in een URL | Reden en bron |
| --- | --- |
| `authkey`, `AUTHKEY`, `auth%6Bey` | [GeoServer AuthKey](https://docs.geoserver.org/stable/en/user/extensions/authkey/index.html) gebruikt deze sleutel om een gebruiker toegang tot beveiligde OGC-diensten te geven. |
| `app_code`, `APP_CODE`, `app%5Fcode` | [HERE App Code](https://docs.here.com/identity-and-access-management/reference/authentication-api-v11-getappcode) beschrijft deze klassieke authenticatie naast `app_id`. De oudere vorm blijft relevant voor opgeslagen exports. |
| `jwt`, `sessionid`, `JSESSIONID`, `PHPSESSID`, `ASP.NET_SessionId` | [OWASP Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html) behandelt authenticatietokens en sessie-id's als gegevens waarmee toegang kan worden overgenomen, en noemt de gebruikelijke sessienamen. |
| `access_key` | De [officiële APILayer-werkruimte](https://www.postman.com/apilayer/apilayer/overview) noemt `access_key` de authenticatieparameter voor onder meer geocodering en schrijft voor de sleutel niet openbaar te maken. |
| `rapidapi-key`, `X-RapidAPI-Key` | [RapidAPI](https://docs.rapidapi.com/docs/keys-and-key-rotation) documenteert de accountsleutel, de headernaam en de queryvorm. Leaflet Providers gebruikt de queryvorm voor MapTilesAPI. |
| `oauth_token`, `oauth_token_secret`, `oauth_signature` | [RFC 5849](https://www.rfc-editor.org/rfc/rfc5849) definieert deze authenticatiegegevens en laat OAuth-parameters in de query toe. De handtekening kan bij de PLAINTEXT-methode zelfs de gedeelde geheimen bevatten. |
| `appid`, uitsluitend bij OpenWeather | [OpenWeather Weather Maps](https://openweathermap.org/api/weathermaps) documenteert `appid` als de vereiste API-sleutel. Dit mag geen algemene blokkade op publieke applicatie-id's worden. |

## Bestaande herkenning met extra dekking

`client_secret`, `signature`, `sig` en `secret` worden al herkend. De auditfixture controleert ze opnieuw. [RFC 6749](https://www.rfc-editor.org/rfc/rfc6749) behandelt `client_secret` als clientwachtwoord. [Google Maps](https://developers.google.com/maps/documentation/maps-static/digital-signature) beschrijft het ondertekeningsgeheim en de authenticatiehandtekening; [MapTiler](https://docs.maptiler.com/guides/developer/credentials/service-token/) beschrijft de overeenkomstige ondertekening. De bestaande token-, API-key- en Azure-controles blijven eveneens staan.

## Key standaard blokkeren, met één publieke uitzondering

De regel uit reviewronde 2 liet `key` op onbekende hosts toe. Daarmee lekt de [Ordnance Survey-URL](https://docs.os.uk/os-apis/core-concepts/authentication) uit ronde 3: `https://api.os.uk/maps/raster/v1/zxy/Outdoor_3857/{z}/{x}/{y}.png?key=REVIEW_SECRET_55`. Ordnance Survey documenteert `key` als de project-API-sleutel en schrijft voor deze als een wachtwoord te behandelen. De fixture bevat deze URL letterlijk, met `key`, `KEY` en `%6Bey`.

Het aangescherpte testcontract blokkeert daarom `key` standaard op iedere host. Een onbekende publieke dienst met zo'n parameter wordt ook geblokkeerd: hier krijgt het voorkomen van een gelekte sleutel voorrang boven het voorkomen van een onterechte blokkade. De bestaande authenticatiemelding blijft gelijk. Er is geen lijst met alle diensten met geheime keys meer nodig.

De uitzonderingslijst bevat alleen de exacte host `taginfo.openstreetmap.org`. [De Taginfo-API](https://taginfo.openstreetmap.org/taginfo/apidoc) gebruikt `key=highway` voor een openbare kaartafbeelding; de waarde is een OSM-tagnaam. Hoofdletters in de host, een afsluitende DNS-punt, een poort en de bestaande normalisatie van parameternamen veranderen die uitzondering niet. Een gelijkende hostnaam, een willekeurig subdomein of de naam Taginfo in een querywaarde geeft geen uitzondering. Andere authenticatieparameters en userinfo blijven ook bij Taginfo geblokkeerd. De tests leggen deze grenzen vast. De uitzondering betekent geen claim dat de Taginfo-afbeelding een WMS-dienst is.

De bestaande kaart-API-gevallen van MapTiler, TomTom, CARTO, Google Maps, Bing Maps en Amazon Location blijven geblokkeerd. De MapTiler-, TomTom- en CARTO-URL's staan in Leaflet Providers. [MapTiler](https://docs.maptiler.com/cloud/api/open-api/), [Google Map Tiles](https://developers.google.com/maps/documentation/tile/get-api-key), [Bing Maps](https://learn.microsoft.com/en-us/bingmaps/rest-services/imagery/get-imagery-metadata) en [Amazon Location](https://docs.aws.amazon.com/location/latest/APIReference/API_geomaps_GetTile.html) documenteren hun API-authenticatie.

## Appid ongewijzigd na gerichte controle

De opnieuw gecontroleerde Leaflet Providers-broncode gebruikt `appid` als API-sleutel alleen bij OpenWeather. [OpenWeather](https://openweathermap.org/appid) noemt de waarde een API-sleutel en publiceert eindpunten onder `openweathermap.org`; de bestaande controle dekt die host en zijn subdomeinen. De vier OpenWeather-varianten blijven in de audit staan. [HERE](https://docs.here.com/identity-and-access-management/reference/authentication-api-v11-getappcode) gebruikt een applicatie-id naast de al geblokkeerde `app_code` voor klassieke authenticatie. De publieke identifier alleen wordt niet als geheim behandeld.

Een gerichte bronzoektocht naar andere kaartdiensten met een geheime `appid` leverde geen tweede concrete reproductie op. Daarom wordt deze regel in ronde 3 niet omgekeerd. Dat is een begrensde broncontrole, geen bewijs dat iedere willekeurige dienst of proxy veilig is.

## Bewust geen algemene blokkade

- `key`: krijgt juist wel een algemene blokkade, met uitsluitend de hierboven onderbouwde Taginfo-uitzondering.
- `app_id` / `appid`: kan een publieke applicatie-id zijn; alleen OpenWeather gebruikt de geteste vorm als API-sleutel. HERE met uitsluitend een publieke `app_id` blijft toegankelijk.
- `client_id`: is volgens RFC 6749 een publieke clientidentifier, geen zelfstandig geheim.
- `code`, `state`, `nonce`: zijn op zichzelf onvoldoende om authenticatie vast te stellen en kunnen gewone selectie- of correlatiewaarden zijn. Er is geen algemene patroonregel op deze namen toegevoegd.
- `session`: [Google Map Tiles](https://developers.google.com/maps/documentation/tile/session_tokens) gebruikt dit voor gedeelde weergaveopties; het is niet hetzelfde als een authenticatiesessie-id.
- `oauth_consumer_key`, `oauth_signature_method`, `oauth_nonce`, `oauth_timestamp`: publieke identifiers en protocolmetadata worden niet door een brede `oauth_`-regel geblokkeerd; de echte token- en handtekeningvelden wel.

De overige herkenning richt zich op de geverifieerde namen. De conservatieve standaard voor `key` geldt ook voor onbekende servers; willekeurige andere parameternamen worden daarmee niet automatisch als geheim bestempeld.
