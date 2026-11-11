-- Zelfgemaakt leesvoorbeeld, verzonnen gegevens, GPL-3.0-only.
CREATE TABLE gpkg_contents (table_name TEXT, data_type TEXT);
CREATE TABLE gpkg_geometry_columns (table_name TEXT, column_name TEXT, srs_id INTEGER);
CREATE TABLE gpkg_spatial_ref_sys (srs_id INTEGER, organization TEXT, organization_coordsys_id INTEGER);
CREATE TABLE a_mix (id INTEGER, geom BLOB);
CREATE TABLE b_good (id INTEGER, geom BLOB);
INSERT INTO gpkg_contents VALUES ('a_mix','features'), ('b_good','features');
INSERT INTO gpkg_geometry_columns VALUES ('a_mix','geom',2056), ('b_good','geom',2056);
INSERT INTO gpkg_spatial_ref_sys VALUES (2056,'EPSG',2056);
INSERT INTO a_mix VALUES (1,X'47500001080800000102000000020000000000000020d6434100000000804f32410000000025d6434100000000944f3241'), (2,X'475000010808000001020000000200000000'), (3,X'475000010808000001010000000000002020d6434100000080804f3241');
INSERT INTO b_good VALUES (1,X'475000010808000001010000000000002020d6434100000080804f3241');
