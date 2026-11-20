-- The schema comes from DemoDB.init_db. These rows and payloads are synthetic.
INSERT INTO demo_files
    (id, path, filename, display_name, file_size, status, added_at, map_name, total_rounds,
     team_a_score, team_b_score, team_a_name, team_b_name, duration_mins, match_date,
     match_date_source, source, remark, content_md5, origin_zip)
VALUES
    (1, 'C:\Fixture\round-one.dem', 'Round_50% Q''uote.dem', '北区 Archive', 150, 'loaded',
     '2025-03-21T10:00:00Z', 'de_荒漠', 20, 13, 7, '甲', '乙', 24.5, '2025-03-18T23:59:59Z',
     'match', 'manual', '保留', 'fixture-md5-one', 'C:\Fixture\archive.zip'),
    (2, 'C:\Fixture\alpha.dem', 'Alpha.dem', NULL, 200, 'done',
     '2025-03-20T20:00:00Z', 'de_mirage', 22, 9, 13, NULL, NULL, 37.5, '2025-03-19T00:00:00Z',
     'match', 'watch', NULL, NULL, NULL),
    (3, 'C:\Fixture\no-date.dem', 'NoDate.dem', NULL, 10, 'loaded',
     '2025-03-19T00:00:00Z', 'de_dust2', 9, NULL, NULL, NULL, NULL, 9.5, NULL,
     NULL, 'manual', NULL, NULL, NULL),
    (4, 'C:\Fixture\multi.dem', 'Multiple.dem', NULL, 150, 'parsing',
     '2025-03-18T23:59:59Z', 'de_mirage', 16, 13, 3, NULL, NULL, 22.0, '2025-03-18T01:00:00Z',
     'header', 'watch', NULL, NULL, NULL),
    (5, 'C:\Fixture\error.dem', 'Error.dem', NULL, 10, 'error',
     '2025-03-30T00:00:00Z', NULL, NULL, NULL, NULL, NULL, NULL, NULL, '',
     NULL, 'manual', NULL, NULL, NULL),
    (6, 'C:\Fixture\pending.dem', 'Pending.dem', 'Staging', 400, 'pending',
     '2025-03-20T00:00:00Z', 'de_mirage', 18, NULL, NULL, NULL, NULL, 19.0, '2025-03-18T02:00:00Z',
     'header', 'watch', NULL, NULL, NULL);

INSERT INTO match_results (demo_path, result_json, created_at) VALUES
    ('C:\Fixture\alpha.dem', '{"players":{"Q''uote":{"clips":[{"category":"highlight","kill_count":5}]}},"auto_target_player":"Q''uote","unknown":{"emoji":"♞"}}', '2025-03-22T00:00:00Z'),
    ('C:\Fixture\error.dem', 'broken', '2025-03-23T00:00:00Z');

INSERT INTO demo_result_summaries
    (demo_path, clip_count, primary_target, analyzed_targets_json,
     four_k_count, five_k_count, result_created_at, summary_version)
VALUES
    ('C:\Fixture\alpha.dem', 1, 'Q''uote', '["Q''uote",17,null,"玩家"]', 0, 1, '2025-03-22T00:00:00Z', 2),
    ('C:\Fixture\error.dem', 0, NULL, 'broken', 0, 0, '2025-03-23T00:00:00Z', 2);

INSERT INTO demo_player_stats
    (demo_id, demo_path, steam_id64, steam_id, account_id, user_id, player_name,
     normalized_name, team_name, team_number, kills, deaths, assists, kd, indexed_at)
VALUES
    (1, 'C:\Fixture\round-one.dem', '76561198000000123', 'STEAM_1:1:40', '39734400', '12',
     'Q''uote%_玩家', 'q''uote%_玩家', '甲', 2, 17, 5, 8, 3.4, '2025-03-21T00:00:00Z'),
    (1, 'C:\Fixture\round-one.dem', NULL, NULL, NULL, '13',
     'No Steam', 'no steam', '乙', 3, 3, 5, 2, 0.6, '2025-03-21T00:00:00Z'),
    (2, 'C:\Fixture\alpha.dem', NULL, 'fixture:two', '7777', NULL,
     'Q''uote', 'q''uote', NULL, 2, 20, 13, 4, 1.538, '2025-03-22T00:00:00Z'),
    (4, 'C:\Fixture\multi.dem', NULL, 'fixture:four-a', '8888', '2',
     'Same A', 'same a', NULL, 2, 17, 7, 4, 2.429, '2025-03-20T00:00:00Z'),
    (4, 'C:\Fixture\multi.dem', NULL, 'fixture:four-b', '9999', '3',
     'Same B', 'same b', NULL, 3, 17, 7, 4, 2.429, '2025-03-20T00:00:00Z');
