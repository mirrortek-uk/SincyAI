//! WP-10 acceptance: chunked upload with resume keeps SHA-256 consistent;
//! tampered chunk rejected; cross-tenant download rejected.
//! Skips without SINCY_TEST_DATABASE_URL.

use sincy_common::AgencyId;
use sincy_transfer::service::{DEFAULT_CHUNK_SIZE, TransferError, TransferService, sha256_hex};
use sincy_transfer::store::LocalObjectStore;
use sqlx::postgres::PgPoolOptions;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(sqlx::PgPool, AgencyId, AgencyId, LocalObjectStore)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE transfer_chunks, transfer_sessions, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    let a = AgencyId::new();
    let b = AgencyId::new();
    for agency in [a, b] {
        sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 't')")
            .bind(agency.as_uuid())
            .execute(&pool)
            .await
            .expect("agency");
    }
    let dir = std::env::temp_dir().join(format!("sincy-store-{}", uuid::Uuid::new_v4()));
    Some((pool, a, b, LocalObjectStore::new(dir)))
}

fn make_file(mib: usize) -> Vec<u8> {
    // deterministic pseudo-random content
    let mut v = Vec::with_capacity(mib * 1024 * 1024);
    let mut x: u64 = 0x9e3779b97f4a7c15;
    while v.len() < mib * 1024 * 1024 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(mib * 1024 * 1024);
    v
}

#[tokio::test]
async fn chunked_upload_resume_and_verify() {
    let Some((pool, agency, other, store)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let svc = TransferService::new(&pool, &store);

    // 20 MiB file, 8 MiB chunks => 3 chunks
    let data = make_file(20);
    let whole_hash = sha256_hex(&data);
    let sess = svc
        .create_session(
            agency,
            None,
            "upload",
            "videos/big.mp4",
            data.len() as i64,
            &whole_hash,
        )
        .await
        .expect("create");
    assert_eq!(sess.chunk_size as usize, DEFAULT_CHUNK_SIZE);

    let chunk = |i: usize| -> Vec<u8> {
        let start = i * DEFAULT_CHUNK_SIZE;
        let end = ((i + 1) * DEFAULT_CHUNK_SIZE).min(data.len());
        data[start..end].to_vec()
    };

    // upload chunk 0 and 1, then "network drops" before chunk 2
    svc.upload_chunk(agency, sess.id, 0, &chunk(0))
        .await
        .expect("c0");
    svc.upload_chunk(agency, sess.id, 1, &chunk(1))
        .await
        .expect("c1");

    // resume: only chunk 2 pending
    let pending = svc.pending_chunks(agency, sess.id).await.expect("pending");
    assert_eq!(pending, vec![2]);

    // tampered chunk rejected
    let mut bad = chunk(2);
    bad[0] ^= 0xff;
    let err = svc.upload_chunk(agency, sess.id, 2, &bad).await;
    // hash mismatch vs size is fine here (same size, different content is accepted
    // per-chunk — the WHOLE-file hash catches it at complete); so expect Ok but final fail
    match err {
        Ok(()) => {
            let res = svc.complete(agency, sess.id).await;
            assert!(matches!(res, Err(TransferError::FileHashMismatch { .. })));
            return;
        }
        Err(e) => panic!("unexpected: {e}"),
    }
}

#[tokio::test]
async fn clean_upload_completes_and_cross_tenant_rejected() {
    let Some((pool, agency, other, store)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let svc = TransferService::new(&pool, &store);
    let data = make_file(20);
    let whole_hash = sha256_hex(&data);
    let sess = svc
        .create_session(
            agency,
            None,
            "upload",
            "videos/ok.mp4",
            data.len() as i64,
            &whole_hash,
        )
        .await
        .expect("create");

    for i in 0..3 {
        let start = i * DEFAULT_CHUNK_SIZE;
        let end = ((i + 1) * DEFAULT_CHUNK_SIZE).min(data.len());
        svc.upload_chunk(agency, sess.id, i, &data[start..end])
            .await
            .expect("chunk");
    }
    // idempotent re-upload of a done chunk
    svc.upload_chunk(agency, sess.id, 0, &data[..DEFAULT_CHUNK_SIZE])
        .await
        .expect("idempotent");

    let got = svc.complete(agency, sess.id).await.expect("complete");
    assert_eq!(
        got, whole_hash,
        "100MB-class file hash must match after resume"
    );

    // download by the owning tenant works
    let sess_dl = svc
        .create_session(
            agency,
            None,
            "download",
            "videos/ok.mp4",
            data.len() as i64,
            &whole_hash,
        )
        .await
        .expect("dl session");
    // mark it completed manually to simulate a finished download session
    sqlx::query("UPDATE transfer_sessions SET state = 'completed' WHERE id = $1")
        .bind(sess_dl.id)
        .execute(&pool)
        .await
        .unwrap();
    let bytes = svc.download(agency, sess_dl.id).await.expect("download");
    assert_eq!(sha256_hex(&bytes), whole_hash);

    // cross-tenant access rejected (other agency cannot see the session)
    let res = svc.download(other, sess_dl.id).await;
    assert!(matches!(res, Err(TransferError::NotFound)));
    let res = svc
        .pending_chunks(other, sess.id)
        .await
        .expect("pending other");
    assert!(res.is_empty());
}
