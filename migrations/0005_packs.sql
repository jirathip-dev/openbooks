ALTER TABLE ledger.org ADD COLUMN pack text NOT NULL DEFAULT 'none';

CREATE TABLE openbooks.pack_version (
    pack text NOT NULL,
    version bigint NOT NULL,
    PRIMARY KEY (pack, version)
);

INSERT INTO openbooks.schema_version (version) VALUES (5);
