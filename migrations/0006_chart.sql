ALTER TABLE ledger.account
    ADD COLUMN type text NOT NULL DEFAULT 'asset'
        CHECK (type IN ('asset', 'liability', 'equity', 'income', 'expense')),
    ADD COLUMN parent_id bigint,
    ADD COLUMN active boolean NOT NULL DEFAULT true,
    ADD COLUMN currency text CHECK (currency ~ '^[A-Z]{3}$'),
    ADD FOREIGN KEY (org_id, parent_id) REFERENCES ledger.account (org_id, account_id);

ALTER TABLE ledger.account ALTER COLUMN type DROP DEFAULT;

CREATE TABLE ledger.account_role (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    role text NOT NULL,
    from_date date NOT NULL,
    account_id bigint NOT NULL,
    PRIMARY KEY (org_id, role, from_date),
    FOREIGN KEY (org_id, account_id) REFERENCES ledger.account (org_id, account_id)
);

CREATE TABLE ledger.dimension (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    kind_id bigint GENERATED ALWAYS AS IDENTITY,
    slot smallint NOT NULL CHECK (slot BETWEEN 1 AND 3),
    name text NOT NULL CHECK (name <> ''),
    removed boolean NOT NULL DEFAULT false,
    PRIMARY KEY (org_id, kind_id),
    CHECK (slot > 1 OR (name = 'Project' AND NOT removed))
);

CREATE UNIQUE INDEX dimension_slot ON ledger.dimension (org_id, slot) WHERE NOT removed;

INSERT INTO ledger.dimension (org_id, slot, name) SELECT org_id, 1, 'Project' FROM ledger.org;

CREATE TABLE ledger.dimension_value (
    org_id bigint NOT NULL,
    value_id bigint GENERATED ALWAYS AS IDENTITY,
    kind_id bigint NOT NULL,
    code text NOT NULL CHECK (code <> ''),
    name text NOT NULL,
    active boolean NOT NULL DEFAULT true,
    host_ref text,
    PRIMARY KEY (org_id, value_id),
    UNIQUE (org_id, kind_id, value_id),
    UNIQUE (org_id, kind_id, code),
    UNIQUE (org_id, kind_id, host_ref),
    FOREIGN KEY (org_id, kind_id) REFERENCES ledger.dimension (org_id, kind_id)
);

CREATE TABLE ledger.line_dimension (
    org_id bigint NOT NULL,
    entry_id bigint NOT NULL,
    line_no integer NOT NULL,
    kind_id bigint NOT NULL,
    value_id bigint NOT NULL,
    PRIMARY KEY (org_id, entry_id, line_no, kind_id),
    FOREIGN KEY (org_id, entry_id, line_no) REFERENCES ledger.line (org_id, entry_id, line_no),
    FOREIGN KEY (org_id, kind_id, value_id) REFERENCES ledger.dimension_value (org_id, kind_id, value_id)
);

CREATE TRIGGER posted_line_dimension_is_final
    BEFORE INSERT OR UPDATE ON ledger.line_dimension
    FOR EACH ROW EXECUTE FUNCTION ledger.posted_line_is_final();

CREATE TABLE ledger.account_dimension (
    org_id bigint NOT NULL,
    account_id bigint NOT NULL,
    kind_id bigint NOT NULL,
    required boolean NOT NULL,
    PRIMARY KEY (org_id, account_id, kind_id),
    FOREIGN KEY (org_id, account_id) REFERENCES ledger.account (org_id, account_id),
    FOREIGN KEY (org_id, kind_id) REFERENCES ledger.dimension (org_id, kind_id)
);

CREATE FUNCTION ledger.entry_has_required_dimensions() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM ledger.line l
        JOIN ledger.account_dimension r ON r.org_id = l.org_id AND r.account_id = l.account_id AND r.required
        JOIN ledger.dimension k ON k.org_id = r.org_id AND k.kind_id = r.kind_id AND NOT k.removed
        WHERE l.org_id = NEW.org_id AND l.entry_id = NEW.entry_id
          AND NOT EXISTS (SELECT 1 FROM ledger.line_dimension d
            WHERE d.org_id = l.org_id AND d.entry_id = l.entry_id AND d.line_no = l.line_no
              AND d.kind_id = r.kind_id)) THEN
        RAISE EXCEPTION 'entry % has a line without a dimension its account requires', NEW.entry_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER entry_has_required_dimensions
    AFTER INSERT OR UPDATE ON ledger.entry
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW WHEN (NEW.posted_at IS NOT NULL)
    EXECUTE FUNCTION ledger.entry_has_required_dimensions();

CREATE TABLE ledger.terms (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    terms_id bigint GENERATED ALWAYS AS IDENTITY,
    code text NOT NULL CHECK (code <> ''),
    name text NOT NULL,
    days integer NOT NULL CHECK (days >= 0),
    active boolean NOT NULL DEFAULT true,
    PRIMARY KEY (org_id, terms_id),
    UNIQUE (org_id, code)
);

INSERT INTO openbooks.schema_version (version) VALUES (6);
