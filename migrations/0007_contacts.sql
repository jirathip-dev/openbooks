CREATE TABLE ledger.contact (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    contact_id bigint GENERATED ALWAYS AS IDENTITY,
    code text NOT NULL CHECK (code <> ''),
    name text NOT NULL CHECK (name <> ''),
    kind text NOT NULL CHECK (kind IN ('customer', 'supplier', 'customer_and_supplier', 'employee')),
    address text,
    email text,
    phone text,
    terms_id bigint,
    currency text CHECK (currency ~ '^[A-Z]{3}$'),
    withholding text,
    tax_id text,
    branch text,
    bank_account text,
    host_ref text,
    PRIMARY KEY (org_id, contact_id),
    UNIQUE (org_id, code),
    UNIQUE (org_id, host_ref),
    FOREIGN KEY (org_id, terms_id) REFERENCES ledger.terms (org_id, terms_id),
    CHECK (kind <> 'employee' OR (terms_id IS NULL AND currency IS NULL AND withholding IS NULL
        AND tax_id IS NULL AND branch IS NULL AND bank_account IS NULL))
);

INSERT INTO openbooks.schema_version (version) VALUES (7);
