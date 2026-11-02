<?xml version="1.0" encoding="UTF-8"?>
<wfs:FeatureCollection xmlns:wfs="http://www.opengis.net/wfs/2.0"
  xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:t="urn:open-infra:test">
  <wfs:member><t:line gml:id="broken-line"><t:geometry>
    <gml:LineString srsName="urn:ogc:def:crs:EPSG::2056" srsDimension="3">
      <gml:posList>2600000 broken 5 2600010 1200020 6</gml:posList>
    </gml:LineString>
  </t:geometry></t:line></wfs:member>
  <wfs:member><t:point gml:id="point-1"><t:geometry>
    <gml:Point srsName="urn:ogc:def:crs:EPSG::2056"><gml:pos>2600005 1200005</gml:pos></gml:Point>
  </t:geometry></t:point></wfs:member>
</wfs:FeatureCollection>
