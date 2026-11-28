SELECT c.relname::text FROM pg_class c
WHERE c.relnamespace = 'ledger'::regnamespace AND c.relkind = 'r'
  AND NOT EXISTS (SELECT 1 FROM pg_attribute a WHERE a.attrelid = c.oid AND a.attname = 'org_id')
UNION ALL SELECT k.conname::text FROM pg_constraint k
WHERE k.connamespace = 'ledger'::regnamespace AND k.contype = 'f'
  AND NOT EXISTS (SELECT 1 FROM pg_attribute a
    WHERE a.attrelid = k.conrelid AND a.attnum = ANY (k.conkey) AND a.attname = 'org_id')
