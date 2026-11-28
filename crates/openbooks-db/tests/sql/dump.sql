SELECT string_agg(item, E'\n' ORDER BY item) FROM (
    SELECT format('column %s.%s.%s %s %s %s', table_schema, table_name, column_name, data_type,
        is_nullable, column_default) AS item
    FROM information_schema.columns WHERE table_schema IN ('ledger', 'openbooks')
    UNION ALL SELECT format('constraint %s %s %s', conrelid::regclass, conname, pg_get_constraintdef(oid))
    FROM pg_constraint WHERE connamespace IN ('ledger'::regnamespace, 'openbooks'::regnamespace)
    UNION ALL SELECT pg_get_triggerdef(oid) FROM pg_trigger
    WHERE NOT tgisinternal AND tgrelid::regclass::text LIKE 'ledger.%'
    UNION ALL SELECT pg_get_functiondef(oid) FROM pg_proc WHERE pronamespace = 'ledger'::regnamespace
    UNION ALL SELECT indexdef FROM pg_indexes WHERE schemaname IN ('ledger', 'openbooks')
    UNION ALL SELECT format('version %s', version) FROM openbooks.schema_version
) AS dump
