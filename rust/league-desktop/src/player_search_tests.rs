use super::*;
use serde_json::json;

const OWN_PUUID: &str = "a0000001-0000-4000-8000-000000000001";
const TARGET_PUUID: &str = "a0000002-0000-4000-8000-000000000002";

fn chat_fixture(platform_id: &str, puuid: &str) -> ChatIdentity {
    // Synthetic values following Akari's ChatPerson platformId and puuid fields.
    serde_json::from_value(json!({"platformId":platform_id,"puuid":puuid})).unwrap()
}

#[test]
fn recognized_platforms_match_pinned_akari_server_keys() {
    // Expected alias outputs replayed from the pinned shared/utils/sgp.ts.
    let own = Puuid::parse(OWN_PUUID).unwrap();
    for (platform, expected) in [
        ("NA", "NA1"),
        ("OCE", "OC1"),
        ("EUW1", "EUW"),
        ("JP1", "JP"),
        ("HN1", "TENCENT_HN1"),
        ("TENCENT_NJ100", "TENCENT_NJ100"),
        ("BGP2", "TENCENT_BGP2"),
    ] {
        let shard = shard_from_chat(Some(chat_fixture(platform, OWN_PUUID)), &own).unwrap();
        assert_eq!(shard.sgp_server_id, expected);
        assert_eq!(shard.region == "TENCENT", expected.starts_with("TENCENT_"));
    }
}

#[test]
fn requires_a_recognized_shard_for_the_same_signed_in_player() {
    let own = Puuid::parse(OWN_PUUID).unwrap();
    assert_eq!(
        shard_from_chat(None, &own),
        Err(PlayerSearchError::ShardUnavailable)
    );
    assert_eq!(
        shard_from_chat(Some(chat_fixture("HN1", TARGET_PUUID)), &own),
        Err(PlayerSearchError::SessionChanged)
    );
    for code in ["", "TENCENT", "TENCENT_UNKNOWN", "staging.na"] {
        assert_eq!(
            shard_from_chat(Some(chat_fixture(code, OWN_PUUID)), &own),
            Err(PlayerSearchError::ShardUnavailable)
        );
    }
    assert!(Puuid::parse("").is_none());
    let shard = shard_from_chat(Some(chat_fixture("HN1", OWN_PUUID)), &own).unwrap();
    assert!(require_requested_shard(&shard, None).is_ok());
    assert!(require_requested_shard(&shard, Some("TENCENT_HN1")).is_ok());
    assert_eq!(
        require_requested_shard(&shard, Some("TENCENT_HN10")),
        Err(PlayerSearchError::OtherShard)
    );
}

#[test]
fn only_exact_names_and_local_uuid_inputs_build_fixed_loopback_requests() {
    let transport = LcuTransport::new().unwrap();
    let target = LcuTarget::new(31111, "fixture-only").unwrap();
    let query = LookupQuery::parse("  测试名&其+他 # CN1  ").unwrap();
    let LookupQuery::Alias {
        game_name,
        tag_line,
    } = query
    else {
        panic!("fixture must parse as an exact Riot ID");
    };
    let request = transport
        .player_request(
            &target,
            PlayerRead::Alias {
                game_name,
                tag_line,
            },
        )
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(request.url().scheme(), "https");
    assert_eq!(request.url().host_str(), Some("127.0.0.1"));
    assert_eq!(request.url().port(), Some(31111));
    assert_eq!(request.url().path(), "/lol-summoner/v1/alias/lookup");
    assert!(request.url().fragment().is_none());
    assert_eq!(
        request.url().query_pairs().into_owned().collect::<Vec<_>>(),
        [
            ("gameName".to_string(), "测试名&其+他".to_string()),
            ("tagLine".to_string(), "CN1".to_string()),
        ]
    );
    assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());

    let LookupQuery::Puuid(puuid) = LookupQuery::parse(TARGET_PUUID).unwrap() else {
        panic!("fixture must parse as a PUUID");
    };
    let request = transport
        .player_request(&target, PlayerRead::Summoner(&puuid))
        .build()
        .unwrap();
    assert_eq!(
        request.url().path(),
        format!("/lol-summoner/v2/summoners/puuid/{TARGET_PUUID}")
    );
    for input in [
        "",
        "Incomplete",
        "Name#",
        "#EUW",
        "Name#TAG#OTHER",
        "Name\n#CN1",
        "../../foo",
        "00000000-0000-0000-0000-000000000000",
    ] {
        assert!(LookupQuery::parse(input).is_err(), "{input:?}");
    }
}

#[test]
fn only_the_requested_players_summary_is_forwarded() {
    let own = Puuid::parse(OWN_PUUID).unwrap();
    let shard = shard_from_chat(Some(chat_fixture("HN1", OWN_PUUID)), &own).unwrap();
    let original = json!({
        "puuid":TARGET_PUUID,"gameName":"示例玩家","tagLine":"CN1","profileIconId":77,
        "privacy":"PUBLIC","summonerLevel":49,"accountId":42,"internalName":"fixture-internal",
    });
    let expected = Puuid::parse(TARGET_PUUID).unwrap();
    let result = project_summoner(
        serde_json::from_value(original.clone()).unwrap(),
        &expected,
        &shard,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({
            "puuid":TARGET_PUUID,"gameName":"示例玩家","tagLine":"CN1","profileIconId":77,
            "privacy":"PUBLIC","summonerLevel":49,"sgpServerId":"TENCENT_HN1",
        })
    );
    assert_eq!(
        project_summoner(serde_json::from_value(original).unwrap(), &own, &shard),
        Err(PlayerSearchError::InvalidResult)
    );
    let alias: AliasResponse = serde_json::from_value(
        json!({"alias":{"gameName":"示例玩家","tagLine":"CN1"},"puuid":TARGET_PUUID}),
    )
    .unwrap();
    assert_eq!(Puuid::parse(&alias.puuid).unwrap().0, TARGET_PUUID);
}
