CREATE SCHEMA openbooks;

CREATE TABLE openbooks.schema_version (
    version bigint PRIMARY KEY
);

CREATE SCHEMA ledger;

CREATE TABLE ledger.org (
    org_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name text NOT NULL,
    base_currency text NOT NULL CHECK (base_currency ~ '^[A-Z]{3}$')
);

CREATE TABLE ledger.account (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    account_id bigint GENERATED ALWAYS AS IDENTITY,
    code text NOT NULL,
    name text NOT NULL,
    PRIMARY KEY (org_id, account_id),
    UNIQUE (org_id, code)
);

CREATE TABLE ledger.entry (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    entry_id bigint GENERATED ALWAYS AS IDENTITY,
    book text NOT NULL CHECK (book IN ('general', 'receivables', 'payables', 'projects')),
    entry_date date NOT NULL,
    memo text NOT NULL DEFAULT '',
    posted_at timestamptz,
    PRIMARY KEY (org_id, entry_id)
);

CREATE TABLE ledger.line (
    org_id bigint NOT NULL,
    entry_id bigint NOT NULL,
    line_no integer NOT NULL CHECK (line_no > 0),
    account_id bigint NOT NULL,
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    amount_minor bigint NOT NULL,
    base_minor bigint NOT NULL,
    PRIMARY KEY (org_id, entry_id, line_no),
    FOREIGN KEY (org_id, entry_id) REFERENCES ledger.entry (org_id, entry_id),
    FOREIGN KEY (org_id, account_id) REFERENCES ledger.account (org_id, account_id)
);

CREATE FUNCTION ledger.entry_balances() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF (SELECT coalesce(sum(base_minor), 0) FROM ledger.line
        WHERE org_id = NEW.org_id AND entry_id = NEW.entry_id) <> 0 THEN
        RAISE EXCEPTION 'entry % does not balance in base currency', NEW.entry_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER entry_balances
    AFTER INSERT OR UPDATE ON ledger.entry
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW WHEN (NEW.posted_at IS NOT NULL)
    EXECUTE FUNCTION ledger.entry_balances();

CREATE FUNCTION ledger.posted_entry_is_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.posted_at IS NOT NULL THEN
        RAISE EXCEPTION 'entry % is posted and never changes', OLD.entry_id
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER posted_entry_is_final
    BEFORE UPDATE ON ledger.entry
    FOR EACH ROW EXECUTE FUNCTION ledger.posted_entry_is_final();

CREATE FUNCTION ledger.posted_line_is_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM ledger.entry
        WHERE posted_at IS NOT NULL
          AND ((org_id = NEW.org_id AND entry_id = NEW.entry_id)
            OR (TG_OP = 'UPDATE' AND org_id = OLD.org_id AND entry_id = OLD.entry_id))
        FOR SHARE) THEN
        RAISE EXCEPTION 'entry % is posted: its lines never change', NEW.entry_id
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER posted_line_is_final
    BEFORE INSERT OR UPDATE ON ledger.line
    FOR EACH ROW EXECUTE FUNCTION ledger.posted_line_is_final();

INSERT INTO openbooks.schema_version (version) VALUES (1);
