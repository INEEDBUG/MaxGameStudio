import pytest
from app.cosmetic_studio.service import validate_capture_audio
from app.cosmetic_studio.provider import CosmeticError

@pytest.mark.parametrize('streams,diagnostic', [([{'codec_type':'video'}],b'max_volume: -1.0 dB'),
    ([{'codec_type':'audio'}],b'max_volume: -inf dB'),
    ([{'codec_type':'audio'}],b''),([{'codec_type':'audio'}],b'max_volume: -90 dB')])
def test_silent_or_unknown_audio_is_not_accepted(streams,diagnostic):
    with pytest.raises(CosmeticError):validate_capture_audio({'streams':streams},diagnostic)

def test_actual_non_silent_audio_content_accepted():
    validate_capture_audio({'streams':[{'codec_type':'video'},{'codec_type':'audio'}]},b'mean_volume: -19.4 dB\nmax_volume: -0.6 dB')

def test_one_peak_over_near_silent_track_does_not_qualify():
    with pytest.raises(CosmeticError):
        validate_capture_audio({'streams':[{'codec_type':'audio'}]},b'mean_volume: -89.0 dB\nmax_volume: -1.0 dB')

@pytest.mark.parametrize('metadata',[
    {'format':{'duration':'2.5'},'streams':[{'codec_type':'audio'}]},
    {'format':{'duration':'NaN'},'streams':[{'codec_type':'video','width':1280,'height':720}]},
    {'format':{'duration':'2.5'},'streams':[{'codec_type':'video','width':640,'height':360}]},
    {'format':{'duration':'41'},'streams':[{'codec_type':'video','width':1280,'height':720}]},
])
def test_unknown_or_wrong_final_video_metadata_cannot_qualify(metadata):
    from app.cosmetic_studio.service import validate_capture_video
    with pytest.raises(CosmeticError):validate_capture_video(metadata)

def test_expected_native_video_metadata_accepted():
    from app.cosmetic_studio.service import validate_capture_video
    assert validate_capture_video({'format':{'duration':'2.5'},
        'streams':[{'codec_type':'video','width':1280,'height':720}]})==2.5
