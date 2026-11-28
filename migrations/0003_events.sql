CREATE TABLE ledger.event (
    org_id bigint NOT NULL REFERENCES ledger.org (org_id),
    event_id bigint GENERATED ALWAYS AS IDENTITY,
    audit_id text NOT NULL,
    action text NOT NULL,
    detail text NOT NULL,
    origin text,
    delivered boolean NOT NULL DEFAULT false,
    PRIMARY KEY (org_id, event_id),
    UNIQUE (org_id, audit_id),
    FOREIGN KEY (org_id, audit_id) REFERENCES ledger.audit (org_id, audit_id)
);

CREATE INDEX event_pending ON ledger.event (org_id, event_id) WHERE NOT delivered;

CREATE TABLE openbooks.idempotency (
    user_id bigint NOT NULL,
    key text NOT NULL,
    request text NOT NULL,
    answer text,
    PRIMARY KEY (user_id, key)
);

INSERT INTO openbooks.schema_version (version) VALUES (3);
