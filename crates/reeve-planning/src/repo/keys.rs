use super::*;

#[async_trait]
impl KeyRepo for PgPlanningRepo {
    async fn next_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        prefix: &str,
    ) -> Result<Key, RepoError> {
        let n: i64 = sqlx::query_scalar("INSERT INTO key_sequence (prefix, last) VALUES ($1, 1) ON CONFLICT (prefix) DO UPDATE SET last = key_sequence.last + 1 RETURNING last").bind(prefix).fetch_one(&mut **tx).await?;
        Ok(Key::new(prefix, n as u64).expect("prefix validated by config"))
    }
    async fn find_by_idempotency(
        &self,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> = sqlx::query_scalar(&format!(
            "SELECT key FROM {table} WHERE idempotency_key = $1"
        ))
        .bind(idem)
        .fetch_optional(&self.pool)
        .await?;
        Ok(k.as_deref().map(key))
    }
    async fn find_by_idempotency_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> = sqlx::query_scalar(&format!(
            "SELECT key FROM {table} WHERE idempotency_key = $1"
        ))
        .bind(idem)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(k.as_deref().map(key))
    }
    async fn key_exists(&self, item: ItemType, k: &Key) -> Result<bool, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        Ok(sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM {table} WHERE key = $1)"
        ))
        .bind(k.to_string())
        .fetch_one(&self.pool)
        .await?)
    }
    async fn item_type_of_key(&self, k: &Key) -> Result<Option<ItemType>, RepoError> {
        let t: Option<String> = sqlx::query_scalar(
            "SELECT t FROM (\
               SELECT 'story' AS t, 0 AS o FROM stories WHERE key = $1 \
               UNION ALL SELECT 'epic', 1 FROM epics WHERE key = $1 \
               UNION ALL SELECT 'initiative', 2 FROM initiatives WHERE key = $1 \
             ) x ORDER BY o LIMIT 1",
        )
        .bind(k.to_string())
        .fetch_optional(&self.pool)
        .await?;
        t.map(|t| match t.as_str() {
            "story" => Ok(ItemType::Story),
            "epic" => Ok(ItemType::Epic),
            "initiative" => Ok(ItemType::Initiative),
            other => Err(bad("item type", other).into()),
        })
        .transpose()
    }
    async fn item_types_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<HashMap<Key, ItemType>, RepoError> {
        item_types_in(tx, keys).await
    }
    async fn find_by_external_ref(
        &self,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> =
            sqlx::query_scalar(&format!("SELECT key FROM {table} WHERE external_ref = $1"))
                .bind(external_ref)
                .fetch_optional(&self.pool)
                .await?;
        Ok(k.as_deref().map(key))
    }
    async fn find_by_external_ref_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> =
            sqlx::query_scalar(&format!("SELECT key FROM {table} WHERE external_ref = $1"))
                .bind(external_ref)
                .fetch_optional(&mut **tx)
                .await?;
        Ok(k.as_deref().map(key))
    }
    async fn lock_external_ref(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        external_ref: &str,
    ) -> Result<(), RepoError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_import:' || $1))")
            .bind(external_ref)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}

async fn item_types_in(tx: Tx<'_, '_>, keys: &[Key]) -> Result<HashMap<Key, ItemType>, RepoError> {
    let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    let rows = sqlx::query(
        "SELECT DISTINCT ON (key) key, t FROM (\
           SELECT key, 'story' AS t, 0 AS o FROM stories WHERE key = ANY($1) \
           UNION ALL SELECT key, 'epic', 1 FROM epics WHERE key = ANY($1) \
           UNION ALL SELECT key, 'initiative', 2 FROM initiatives WHERE key = ANY($1) \
         ) x ORDER BY key, o",
    )
    .bind(&keys)
    .fetch_all(&mut **tx)
    .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let t = match r.get::<&str, _>("t") {
            "story" => ItemType::Story,
            "epic" => ItemType::Epic,
            "initiative" => ItemType::Initiative,
            other => return Err(bad("item type", other).into()),
        };
        out.insert(key(r.get("key")), t);
    }
    Ok(out)
}
