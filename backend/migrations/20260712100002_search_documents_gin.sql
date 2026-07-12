-- no-transaction
-- Full-text index for Global Search. Built CONCURRENTLY (the
-- activity_feed_index recipe) so a production deployment keeps serving
-- while it builds — which is why this migration opts out of sqlx's
-- transaction AND contains exactly one statement: PostgreSQL runs a
-- multi-statement simple-query batch inside an implicit transaction,
-- where CREATE INDEX CONCURRENTLY is rejected.
--
-- IF NOT EXISTS keeps a boot-time retry from hard-blocking startup. If a
-- concurrent build ever crashes mid-flight and leaves an INVALID index,
-- remediation is `DROP INDEX search_documents_search_gin;` + restart.
CREATE INDEX CONCURRENTLY IF NOT EXISTS search_documents_search_gin
    ON search_documents USING GIN (search);
