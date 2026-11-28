ALTER TABLE ledger.audit DROP CONSTRAINT audit_via_check;

ALTER TABLE ledger.audit
    ADD CONSTRAINT audit_via_check CHECK (via IN ('person', 'assistant', 'host', 'pack', 'schedule')),
    ADD COLUMN seq bigint GENERATED ALWAYS AS IDENTITY,
    ADD COLUMN record text NOT NULL DEFAULT '',
    ADD COLUMN before jsonb,
    ADD COLUMN after jsonb;

ALTER TABLE ledger.audit ALTER COLUMN record DROP DEFAULT;

CREATE UNIQUE INDEX audit_in_order ON ledger.audit (org_id, seq);

CREATE FUNCTION ledger.audit_is_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'an audit row is never changed or removed'
        USING ERRCODE = 'integrity_constraint_violation';
END
$$;

CREATE TRIGGER audit_row_is_final
    BEFORE UPDATE OR DELETE ON ledger.audit
    FOR EACH ROW EXECUTE FUNCTION ledger.audit_is_final();

CREATE TRIGGER audit_is_never_emptied
    BEFORE TRUNCATE ON ledger.audit
    FOR EACH STATEMENT EXECUTE FUNCTION ledger.audit_is_final();

INSERT INTO openbooks.schema_version (version) VALUES (4);
