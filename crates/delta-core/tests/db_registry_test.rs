//! Tests for artifact, release, signing, LFS, retention, and ark package DB operations.

mod common;

use delta_core::db;

// --- Artifact tests ---

#[tokio::test]
async fn test_artifact_create_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "build.tar.gz",
            version: Some("1.0.0"),
            artifact_type: "generic",
            content_hash: "abc123",
            size_bytes: 1024,
            metadata: Some("{\"arch\": \"amd64\"}"),
        },
    )
    .await
    .expect("failed to create artifact");

    assert_eq!(artifact.name, "build.tar.gz");
    assert_eq!(artifact.version.as_deref(), Some("1.0.0"));
    assert_eq!(artifact.size_bytes, 1024);
    assert_eq!(artifact.download_count, 0);

    let fetched = db::artifact::get(&pool, &artifact.id).await.unwrap();
    assert_eq!(fetched.id, artifact.id);
    assert_eq!(fetched.content_hash, "abc123");
}

#[tokio::test]
async fn test_artifact_list_for_repo() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    for i in 0..3 {
        db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo.id.to_string(),
                pipeline_id: None,
                name: &format!("artifact-{}", i),
                version: None,
                artifact_type: "generic",
                content_hash: &format!("hash{}", i),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    let artifacts = db::artifact::list_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 3);
}

#[tokio::test]
async fn test_artifact_increment_download() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "file.zip",
            version: None,
            artifact_type: "generic",
            content_hash: "hash1",
            size_bytes: 50,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::artifact::increment_download(&pool, &artifact.id)
        .await
        .unwrap();
    db::artifact::increment_download(&pool, &artifact.id)
        .await
        .unwrap();

    let fetched = db::artifact::get(&pool, &artifact.id).await.unwrap();
    assert_eq!(fetched.download_count, 2);
}

#[tokio::test]
async fn test_artifact_delete() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "to-delete",
            version: None,
            artifact_type: "generic",
            content_hash: "delhash",
            size_bytes: 10,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::artifact::delete(&pool, &artifact.id).await.unwrap();

    let result = db::artifact::get(&pool, &artifact.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_artifact_delete_not_found() {
    let pool = common::setup_pool().await;
    let result = db::artifact::delete(&pool, "nonexistent").await;
    assert!(result.is_err());
}

// --- Release tests ---

#[tokio::test]
async fn test_release_create_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let release = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo.id.to_string(),
            tag_name: "2026.1.1",
            name: "Release 2026.1.1",
            body: Some("First release!"),
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .expect("failed to create release");

    assert_eq!(release.tag_name, "2026.1.1");
    assert_eq!(release.name, "Release 2026.1.1");
    assert_eq!(release.body.as_deref(), Some("First release!"));
    assert!(!release.is_draft);
    assert!(!release.is_prerelease);
}

#[tokio::test]
async fn test_release_get_by_tag() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.2.1",
            name: "2026.2.1",
            body: None,
            is_draft: false,
            is_prerelease: true,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    let release = db::release::get_by_tag(&pool, &repo_id, "2026.2.1")
        .await
        .unwrap();
    assert_eq!(release.tag_name, "2026.2.1");
    assert!(release.is_prerelease);
}

#[tokio::test]
async fn test_release_get_by_tag_not_found() {
    let pool = common::setup_pool().await;
    let result = db::release::get_by_tag(&pool, "norepo", "notag").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_list_for_repo() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..3 {
        db::release::create(
            &pool,
            &db::release::CreateReleaseParams {
                repo_id: &repo_id,
                tag_name: &format!("2026.{}.1", i + 1),
                name: &format!("Release {}", i),
                body: None,
                is_draft: false,
                is_prerelease: false,
                author_id: &user.id.to_string(),
            },
        )
        .await
        .unwrap();
    }

    let releases = db::release::list_for_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(releases.len(), 3);
}

#[tokio::test]
async fn test_release_duplicate_tag_fails() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.3.1",
            name: "First",
            body: None,
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    let result = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.3.1",
            name: "Duplicate",
            body: None,
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_delete() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let release = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.4.1",
            name: "To delete",
            body: None,
            is_draft: true,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    db::release::delete(&pool, &release.id).await.unwrap();
    let result = db::release::get(&pool, &release.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_delete_not_found() {
    let pool = common::setup_pool().await;
    let result = db::release::delete(&pool, "nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_attach_asset() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let release = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.5.1",
            name: "With asset",
            body: None,
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "asset-file",
            version: None,
            artifact_type: "generic",
            content_hash: "assethash",
            size_bytes: 500,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::release::attach_asset(&pool, &release.id, &artifact.id, Some("binary"))
        .await
        .unwrap();
}

// --- Signing key tests ---

#[tokio::test]
async fn test_signing_key_crud() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let key = db::signing::add_signing_key(
        &pool,
        &uid,
        "mykey",
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    )
    .await
    .expect("failed to add signing key");

    assert_eq!(key.name, "mykey");
    assert_eq!(key.user_id, uid);

    let fetched = db::signing::get_signing_key(&pool, &key.id)
        .await
        .expect("failed to get key");
    assert_eq!(fetched.id, key.id);

    let keys = db::signing::list_signing_keys(&pool, &uid).await.unwrap();
    assert_eq!(keys.len(), 1);

    db::signing::delete_signing_key(&pool, &key.id, &uid)
        .await
        .expect("failed to delete");
    let keys = db::signing::list_signing_keys(&pool, &uid).await.unwrap();
    assert!(keys.is_empty());
}

#[tokio::test]
async fn test_signing_key_delete_wrong_user() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let key = db::signing::add_signing_key(
        &pool,
        &uid,
        "mykey",
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    )
    .await
    .unwrap();

    let result = db::signing::delete_signing_key(&pool, &key.id, "wrong-user").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_artifact_signature_crud() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "build.tar.gz",
            version: Some("1.0.0"),
            artifact_type: "generic",
            content_hash: "abc123",
            size_bytes: 1024,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let key = db::signing::add_signing_key(
        &pool,
        &user.id.to_string(),
        "mykey",
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    )
    .await
    .unwrap();

    let sig = db::signing::add_signature(&pool, &artifact.id, &key.id, "deadbeef")
        .await
        .expect("failed to add signature");

    assert_eq!(sig.artifact_id, artifact.id);
    assert_eq!(sig.signer_key_id, key.id);

    let sigs = db::signing::get_signatures(&pool, &artifact.id)
        .await
        .unwrap();
    assert_eq!(sigs.len(), 1);
}

// --- LFS tests ---

#[tokio::test]
async fn test_lfs_create_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let obj = db::lfs::create(&pool, &repo_id, "sha256:abc123", 1024)
        .await
        .expect("failed to create LFS object");

    assert_eq!(obj.oid, "sha256:abc123");
    assert_eq!(obj.size, 1024);

    let fetched = db::lfs::get(&pool, &repo_id, "sha256:abc123")
        .await
        .unwrap();
    assert_eq!(fetched.id, obj.id);
}

#[tokio::test]
async fn test_lfs_exists() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    assert!(
        !db::lfs::exists(&pool, &repo_id, "sha256:nope")
            .await
            .unwrap()
    );

    db::lfs::create(&pool, &repo_id, "sha256:exists123", 512)
        .await
        .unwrap();

    assert!(
        db::lfs::exists(&pool, &repo_id, "sha256:exists123")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_lfs_list_and_delete() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::lfs::create(&pool, &repo_id, "sha256:obj1", 100)
        .await
        .unwrap();
    db::lfs::create(&pool, &repo_id, "sha256:obj2", 200)
        .await
        .unwrap();

    let list = db::lfs::list_by_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(list.len(), 2);

    db::lfs::delete(&pool, &repo_id, "sha256:obj1")
        .await
        .unwrap();

    let list = db::lfs::list_by_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].oid, "sha256:obj2");
}

// --- Retention tests ---

#[tokio::test]
async fn test_retention_policy_set_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let policy = db::retention::get_policy(&pool, &repo_id).await.unwrap();
    assert!(policy.is_none());

    let policy = db::retention::set_policy(
        &pool,
        &db::retention::SetPolicyParams {
            repo_id: &repo_id,
            max_age_days: Some(30),
            max_count: Some(100),
            max_total_bytes: Some(1_000_000),
        },
    )
    .await
    .expect("failed to set policy");

    assert_eq!(policy.max_age_days, Some(30));
    assert_eq!(policy.max_count, Some(100));
    assert_eq!(policy.max_total_bytes, Some(1_000_000));

    let updated = db::retention::set_policy(
        &pool,
        &db::retention::SetPolicyParams {
            repo_id: &repo_id,
            max_age_days: Some(7),
            max_count: None,
            max_total_bytes: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.max_age_days, Some(7));
    assert!(updated.max_count.is_none());
}

#[tokio::test]
async fn test_retention_total_artifact_size() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let total = db::retention::total_artifact_size(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(total, 0);

    db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "a.tar",
            version: None,
            artifact_type: "generic",
            content_hash: "hash1",
            size_bytes: 500,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "b.tar",
            version: None,
            artifact_type: "generic",
            content_hash: "hash2",
            size_bytes: 300,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let total = db::retention::total_artifact_size(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(total, 800);
}

#[tokio::test]
async fn test_retention_find_excess_artifacts() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..5 {
        db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("artifact_{}", i),
                version: None,
                artifact_type: "generic",
                content_hash: &format!("hash_{}", i),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    let excess = db::retention::find_excess_artifacts(&pool, &repo_id, 3)
        .await
        .unwrap();
    assert_eq!(excess.len(), 2);
}

#[tokio::test]
async fn test_retention_find_oversize_artifacts() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..4 {
        db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("art_{}", i),
                version: None,
                artifact_type: "generic",
                content_hash: &format!("h_{}", i),
                size_bytes: 250,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    let oversize = db::retention::find_oversize_artifacts(&pool, &repo_id, 500)
        .await
        .unwrap();
    assert_eq!(oversize.len(), 2);

    let oversize = db::retention::find_oversize_artifacts(&pool, &repo_id, 2000)
        .await
        .unwrap();
    assert!(oversize.is_empty());
}

// --- Ark package tests ---

#[tokio::test]
async fn test_ark_package_publish_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "mylib",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "arkhash1",
            size_bytes: 2048,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let pkg = db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "mylib",
            version: "1.0.0",
            arch: "x86_64",
            description: Some("A test library"),
            dependencies: Some("libc>=2.0"),
            provides: None,
        },
    )
    .await
    .expect("failed to publish");

    assert_eq!(pkg.package_name, "mylib");
    assert_eq!(pkg.version, "1.0.0");
    assert_eq!(pkg.arch, "x86_64");

    let fetched = db::ark_package::get(&pool, &pkg.id).await.unwrap();
    assert_eq!(fetched.package_name, "mylib");
}

#[tokio::test]
async fn test_ark_package_list_versions() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    for ver in ["1.0.0", "1.1.0", "2.0.0"] {
        let artifact = db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("pkg-{}", ver),
                version: Some(ver),
                artifact_type: "ark",
                content_hash: &format!("hash-{}", ver),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();

        db::ark_package::publish(
            &pool,
            &db::ark_package::PublishParams {
                artifact_id: &artifact.id,
                repo_id: &repo_id,
                publisher_id: &uid,
                package_name: "mypkg",
                version: ver,
                arch: "any",
                description: None,
                dependencies: None,
                provides: None,
            },
        )
        .await
        .unwrap();
    }

    let versions = db::ark_package::list_versions(&pool, "mypkg")
        .await
        .unwrap();
    assert_eq!(versions.len(), 3);
}

#[tokio::test]
async fn test_ark_package_get_version() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "pkg-v1",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "vhash1",
            size_bytes: 100,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "versioned",
            version: "1.0.0",
            arch: "any",
            description: None,
            dependencies: None,
            provides: None,
        },
    )
    .await
    .unwrap();

    let pkg = db::ark_package::get_version(&pool, "versioned", "1.0.0", None)
        .await
        .unwrap();
    assert_eq!(pkg.version, "1.0.0");

    let result = db::ark_package::get_version(&pool, "versioned", "9.9.9", None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ark_package_search() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "searchpkg",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "shash1",
            size_bytes: 100,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "awesome-lib",
            version: "1.0.0",
            arch: "any",
            description: Some("An awesome library for testing"),
            dependencies: None,
            provides: None,
        },
    )
    .await
    .unwrap();

    let results = db::ark_package::search(&pool, "awesome", 10, 0)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    let results = db::ark_package::search(&pool, "testing", 10, 0)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    let results = db::ark_package::search(&pool, "nonexistent", 10, 0)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_ark_package_delete() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "delpkg",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "dhash1",
            size_bytes: 100,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let pkg = db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "deleteme",
            version: "1.0.0",
            arch: "any",
            description: None,
            dependencies: None,
            provides: None,
        },
    )
    .await
    .unwrap();

    db::ark_package::delete(&pool, &pkg.id)
        .await
        .expect("delete failed");

    let result = db::ark_package::get(&pool, &pkg.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ark_package_get_latest() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    for (ver, arch) in [
        ("1.0.0", "x86_64"),
        ("1.1.0", "x86_64"),
        ("1.0.0", "aarch64"),
    ] {
        let artifact = db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("latestpkg-{}-{}", ver, arch),
                version: Some(ver),
                artifact_type: "ark",
                content_hash: &format!("lhash-{}-{}", ver, arch),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();

        db::ark_package::publish(
            &pool,
            &db::ark_package::PublishParams {
                artifact_id: &artifact.id,
                repo_id: &repo_id,
                publisher_id: &uid,
                package_name: "latestpkg",
                version: ver,
                arch,
                description: None,
                dependencies: None,
                provides: None,
            },
        )
        .await
        .unwrap();
    }

    let latest = db::ark_package::get_latest(&pool, "latestpkg", None)
        .await
        .unwrap();
    assert_eq!(latest.package_name, "latestpkg");

    let latest = db::ark_package::get_latest(&pool, "latestpkg", Some("aarch64"))
        .await
        .unwrap();
    assert_eq!(latest.arch, "aarch64");

    let result = db::ark_package::get_latest(&pool, "nonexistent", None).await;
    assert!(result.is_err());
}
