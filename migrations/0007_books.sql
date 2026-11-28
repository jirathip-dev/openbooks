CREATE TABLE ledger.book (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    kind text NOT NULL CHECK (kind IN ('general', 'receipts', 'payments', 'purchases', 'sales')),
    code text NOT NULL CHECK (code ~ '^[A-Za-z0-9]+$'),
    position smallint NOT NULL CHECK (position > 0),
    name text NOT NULL CHECK (name <> ''),
    PRIMARY KEY (org_id, kind),
    UNIQUE (org_id, code)
);

CREATE TABLE ledger.series (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    code text NOT NULL CHECK (code ~ '^[A-Za-z0-9]+$'),
    book text,
    document text CHECK (document <> ''),
    year_offset smallint NOT NULL DEFAULT 0 CHECK (year_offset >= 0),
    PRIMARY KEY (org_id, code),
    UNIQUE (org_id, document),
    CHECK (book IS NOT NULL OR document IS NOT NULL),
    FOREIGN KEY (org_id, book) REFERENCES ledger.book (org_id, kind)
);

CREATE UNIQUE INDEX series_of_book ON ledger.series (org_id, book) WHERE document IS NULL;

CREATE TABLE ledger.number (
    org_id bigint NOT NULL,
    code text NOT NULL,
    month date NOT NULL CHECK (extract(day FROM month) = 1),
    seq integer NOT NULL CHECK (seq > 0),
    number text NOT NULL,
    PRIMARY KEY (org_id, code, month, seq),
    UNIQUE (org_id, number),
    FOREIGN KEY (org_id, code) REFERENCES ledger.series (org_id, code)
);

INSERT INTO ledger.book (org_id, kind, code, position, name)
SELECT o.org_id, b.kind, b.code, b.position, b.name FROM ledger.org o
CROSS JOIN (VALUES ('general', 'JV', 1, 'General journal'), ('receipts', 'RV', 2, 'Receipts'),
    ('payments', 'PV', 3, 'Payments'), ('purchases', 'AP', 4, 'Purchases'), ('sales', 'AR', 5, 'Sales'))
    AS b (kind, code, position, name);

INSERT INTO ledger.series (org_id, code, book) SELECT org_id, code, kind FROM ledger.book;

INSERT INTO openbooks.schema_version (version) VALUES (7);
