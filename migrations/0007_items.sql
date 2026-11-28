CREATE TABLE ledger.unit (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    unit_id bigint GENERATED ALWAYS AS IDENTITY,
    code text NOT NULL CHECK (code <> ''),
    name text NOT NULL,
    PRIMARY KEY (org_id, unit_id),
    UNIQUE (org_id, code)
);

CREATE TABLE ledger.item (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    item_id bigint GENERATED ALWAYS AS IDENTITY,
    code text NOT NULL CHECK (code <> ''),
    name text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('stock', 'non_stock', 'service')),
    unit_id bigint NOT NULL,
    sales_account_id bigint,
    purchase_account_id bigint,
    sales_tax text,
    purchase_tax text,
    weight_e4 bigint CHECK (weight_e4 >= 0),
    volume_e4 bigint CHECK (volume_e4 >= 0),
    host_ref text,
    PRIMARY KEY (org_id, item_id),
    UNIQUE (org_id, code),
    UNIQUE (org_id, host_ref),
    FOREIGN KEY (org_id, unit_id) REFERENCES ledger.unit (org_id, unit_id),
    FOREIGN KEY (org_id, sales_account_id) REFERENCES ledger.account (org_id, account_id),
    FOREIGN KEY (org_id, purchase_account_id) REFERENCES ledger.account (org_id, account_id)
);

ALTER TABLE ledger.line
    ADD COLUMN item_id bigint,
    ADD FOREIGN KEY (org_id, item_id) REFERENCES ledger.item (org_id, item_id);

INSERT INTO openbooks.schema_version (version) VALUES (7);
