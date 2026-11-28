INSERT INTO ledger.org (name, base_currency) VALUES ('Acme', 'USD');
INSERT INTO ledger.account (org_id, code, name) VALUES (1, '1000', 'Cash'), (1, '4000', 'Sales');
INSERT INTO ledger.entry (org_id, book, entry_date) VALUES (1, 'general', '2026-11-01');
INSERT INTO ledger.line VALUES (1, 1, 1, 1, 'USD', 500, 499), (1, 1, 2, 2, 'USD', -500, -500);
