CREATE TABLE ledger.member (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    email text NOT NULL CHECK (email = lower(email)),
    user_id bigint,
    role text NOT NULL CHECK (role IN ('owner', 'accountant', 'staff', 'viewer')),
    removed boolean NOT NULL DEFAULT false,
    PRIMARY KEY (org_id, email),
    UNIQUE (org_id, user_id)
);

CREATE TABLE ledger.audit (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    audit_id text NOT NULL,
    at timestamptz NOT NULL,
    user_id bigint NOT NULL,
    via text NOT NULL CHECK (via IN ('person', 'assistant', 'host')),
    action text NOT NULL,
    detail text NOT NULL,
    PRIMARY KEY (org_id, audit_id)
);

INSERT INTO openbooks.schema_version (version) VALUES (2);
