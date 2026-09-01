from app.ai_reviewer import select_reviewer_prompt, select_meme_montage_prompt


def test_default_locale_is_chinese():
    assert "中文" in select_reviewer_prompt("zh")


def test_english_prompt_is_english_and_plain():
    p = select_reviewer_prompt("en")
    assert "中文" not in p
    assert "English" in p


def test_meme_montage_english_prompt():
    assert "English" in select_meme_montage_prompt("en")
    assert "中文" in select_meme_montage_prompt("zh")


def test_regional_prompts_request_the_selected_output_language():
    assert "Bahasa Melayu" in select_reviewer_prompt("ms-MY")
    assert "Russian" in select_meme_montage_prompt("ru-RU")
    assert "中文" in select_reviewer_prompt("zh-TW")
