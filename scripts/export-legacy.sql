-- Contains password hashes and encrypted secrets: redirect stdout to a private file.
-- Run: (umask 077; psql "$DATABASE_URL" -X -A -t -v ON_ERROR_STOP=1 -f scripts/export-legacy.sql > legacy.json)
-- One statement gives every exported model the same PostgreSQL snapshot.
SELECT jsonb_build_object(
  'format', 'subboost-postgresql',
  'version', 1,
  'admins', COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) FROM "LocalAdmin" a), '[]'::jsonb),
  'subscriptions', COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY s.id) FROM "Subscription" s), '[]'::jsonb),
  'templates', COALESCE((SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM "LocalTemplate" t), '[]'::jsonb),
  'autoUpdateStates', COALESCE((SELECT jsonb_agg(to_jsonb(u) ORDER BY u."subscriptionId") FROM "SubscriptionAutoUpdateState" u), '[]'::jsonb),
  'sourceSettings', COALESCE((SELECT jsonb_agg(jsonb_build_object(
    'ownerId', a.id,
    'allowUnsafeSubscriptionSources', COALESCE(to_jsonb(a)->'allowUnsafeSubscriptionSources', 'false'::jsonb)
  ) ORDER BY a.id) FROM "LocalAdmin" a), '[]'::jsonb)
);
-- Sessions, revoked-session records and transient job leases are intentionally not exported.
