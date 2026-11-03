# Authenticatieparameters in QGIS-verbindingen

De broncontrole van 3 november 2026 dekt de zes letterlijke GeoServer- en HERE-reproducties en de gangbare WMS-/XYZ-parameters uit de [Leaflet Providers-broncode](https://github.com/leaflet-extras/leaflet-providers/blob/master/leaflet-providers.js). De gelezen bestandsblob is `6574503328dce05991f24db86bcd40d76dbab6bc`. Daarnaast zijn de onderstaande primaire documentatie en publieke tegenvoorbeelden gecontroleerd.

De tests maken geen netwerkverbinding. De URL's zijn invoer voor de authenticatiecontrole; deze tests bewijzen niet dat ieder eindpunt een WMS of een nog werkende tegeldienst aanbiedt. Alle waarden zijn verzonnen. De zes reviewgevallen en de vijftig auditgevallen gaan door parser, echte egui-voorvertoning en echte opslag in een tijdelijk `servers.json`.

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

## De naam key beperken tot bekende authenticatiecontext

Een algemene blokkade op `key` is te breed. [Taginfo](https://taginfo.openstreetmap.org/taginfo/apidoc) gebruikt `key=highway` voor een openbare kaartafbeelding; dit is een OSM-tagnaam en geen wachtwoord.

`key` blijft alleen een authenticatieparameter op de gecontroleerde kaart-API's van MapTiler, TomTom, CARTO, Google Maps, Bing Maps en Amazon Location. De MapTiler-, TomTom- en CARTO-URL's staan letterlijk in Leaflet Providers. [MapTiler](https://docs.maptiler.com/cloud/api/open-api/) noemt `key` de API-authenticatieparameter. [Google Map Tiles](https://developers.google.com/maps/documentation/tile/get-api-key), [Bing Maps](https://learn.microsoft.com/en-us/bingmaps/rest-services/imagery/get-imagery-metadata) en [Amazon Location](https://docs.aws.amazon.com/location/latest/APIReference/API_geomaps_GetTile.html) documenteren dezelfde rol. De [AWS-eindpunten](https://docs.aws.amazon.com/general/latest/gr/location.html) onderbouwen de regionale domeinen.

De tests houden een publieke `key`, hoofdletter- en percentvarianten, gelijkende domeinnamen en een providernaam die alleen als querywaarde voorkomt toegankelijk. Bekende API-authenticatie blijft geblokkeerd. Dit zegt niet dat iedere API-sleutel altijd privé is: MapTiler staat bijvoorbeeld publicatie na het aanbrengen van beperkingen toe. De import ondersteunt hier nog steeds geen authenticatie en kan uit een sleutel niet afleiden welke beperkingen zijn ingesteld.

## Bewust geen algemene blokkade

- `key`: kan een openbare OSM-tag of kaartselectie zijn; alleen de gecontroleerde dienstcontexten tellen mee.
- `app_id` / `appid`: kan een publieke applicatie-id zijn; alleen OpenWeather gebruikt de geteste vorm als API-sleutel. HERE met uitsluitend een publieke `app_id` blijft toegankelijk.
- `client_id`: is volgens RFC 6749 een publieke clientidentifier, geen zelfstandig geheim.
- `code`, `state`, `nonce`: zijn op zichzelf onvoldoende om authenticatie vast te stellen en kunnen gewone selectie- of correlatiewaarden zijn. Er is geen algemene patroonregel op deze namen toegevoegd.
- `session`: [Google Map Tiles](https://developers.google.com/maps/documentation/tile/session_tokens) gebruikt dit voor gedeelde weergaveopties; het is niet hetzelfde als een authenticatiesessie-id.
- `oauth_consumer_key`, `oauth_signature_method`, `oauth_nonce`, `oauth_timestamp`: publieke identifiers en protocolmetadata worden niet door een brede `oauth_`-regel geblokkeerd; de echte token- en handtekeningvelden wel.

De herkenning richt zich op de geverifieerde namen. Zelfbedachte parameters van een willekeurige server worden niet alleen vanwege hun naam als geheim bestempeld.
