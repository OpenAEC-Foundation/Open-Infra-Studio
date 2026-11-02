<?xml version="1.0" encoding="UTF-8"?>
<imro:FeatureCollectionIMRO xmlns:imro="http://www.geonovum.nl/imro/2012/1.1" xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:xlink="http://www.w3.org/1999/xlink">
  <gml:featureMember><imro:Bestemmingsplangebied gml:id="plan"><imro:identificatie><imro:NEN3610ID><imro:lokaalID>NL.IMRO.0000.synthetic-TEST</imro:lokaalID></imro:NEN3610ID></imro:identificatie><imro:naam>Synthetisch testplan</imro:naam><imro:planstatusInfo><imro:PlanstatusEnDatum><imro:planstatus>vastgesteld</imro:planstatus><imro:datum>2026-01-01</imro:datum></imro:PlanstatusEnDatum></imro:planstatusInfo><imro:geometrie><gml:Polygon srsName="urn:ogc:def:crs:EPSG::28992"><gml:exterior><gml:LinearRing><gml:posList>100000 400000 100020 400000 100020 400020 100000 400020 100000 400000</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></imro:geometrie></imro:Bestemmingsplangebied></gml:featureMember>
  <gml:featureMember><imro:Enkelbestemming gml:id="zone"><imro:naam>Wonen</imro:naam><imro:bestemmingshoofdgroep>wonen</imro:bestemmingshoofdgroep><imro:verwijzingNaarTekst><imro:TekstReferentie><imro:verwijzingNaarTekst>https://example.invalid/regels.html</imro:verwijzingNaarTekst></imro:TekstReferentie></imro:verwijzingNaarTekst><imro:geometrie xlink:href="#polygon"/></imro:Enkelbestemming></gml:featureMember>
  <gml:featureMember><imro:Bouwvlak gml:id="bouw"><imro:geometrie><gml:Polygon srsName="EPSG:28992"><gml:exterior><gml:LinearRing><gml:posList>100002 400002 100008 400002 100008 400008 100002 400008 100002 400002</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></imro:geometrie></imro:Bouwvlak></gml:featureMember>
  <gml:Polygon gml:id="polygon" srsName="EPSG:28992"><gml:exterior><gml:LinearRing><gml:posList>100001 400001 100019 400001 100019 400019 100001 400019 100001 400001</gml:posList></gml:LinearRing></gml:exterior><gml:interior><gml:LinearRing><gml:posList>100010 400010 100012 400010 100012 400012 100010 400012 100010 400010</gml:posList></gml:LinearRing></gml:interior></gml:Polygon>
  <gml:featureMember>
    <imro:Maatvoering gml:id="maten">
      <imro:naam>maximum bouwhoogte, maximum goothoogte</imro:naam>
      <imro:maatvoeringInfo><imro:WaardeEnType><imro:waarde>10</imro:waarde><imro:waardeType>maximum bouwhoogte (m)</imro:waardeType></imro:WaardeEnType></imro:maatvoeringInfo>
      <imro:maatvoeringInfo><imro:WaardeEnType><imro:waarde>6</imro:waarde><imro:waardeType>maximum goothoogte (m)</imro:waardeType></imro:WaardeEnType></imro:maatvoeringInfo>
      <imro:verwijzingNaarTekstInfo><imro:TekstReferentie_BP><imro:verwijzingNaarTekst>regels.html#artikel-3</imro:verwijzingNaarTekst><imro:typeTekst>regels</imro:typeTekst></imro:TekstReferentie_BP></imro:verwijzingNaarTekstInfo>
      <imro:verwijzingNaarTekstInfo><imro:TekstReferentie_BP><imro:verwijzingNaarTekst>toelichting.html#hoofdstuk-4</imro:verwijzingNaarTekst><imro:typeTekst>toelichting</imro:typeTekst></imro:TekstReferentie_BP></imro:verwijzingNaarTekstInfo>
      <imro:bestemmingsvlak xlink:href="#zone"/>
      <imro:geometrie xlink:href="#polygon"/>
    </imro:Maatvoering>
  </gml:featureMember>
</imro:FeatureCollectionIMRO>
