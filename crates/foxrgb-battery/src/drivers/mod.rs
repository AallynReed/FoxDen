//! Battery drivers, in the order they run. Specific drivers claim their products before the generic fallbacks.

mod airpods;
mod ajazz_ak992;
mod akg;
mod akko;
mod alienware;
mod arbiter;
mod areson;
mod astro;
mod atk_bitmouse;
mod atk_keyboard;
mod atk_serial;
mod audeze;
mod aula;
mod aula_mouse;
mod barracuda_pro;
mod beitong;
mod bt;
mod cherry;
mod chimera;
mod coolermaster;
mod corsair;
mod dareu;
mod darkrim;
mod darmoshark;
mod eightbitdo;
mod endgame;
mod endgame_we;
mod endorfy;
mod eweadn;
mod fantech;
mod finalmouse;
mod flydigi;
mod fractal;
mod galaxy_buds;
mod gamesir;
mod gamesir_ble;
mod gamesir_sapjoy;
mod generic;
mod glorious;
mod hyperx;
mod jabra;
mod jbl;
mod keychron;
mod kysona;
mod lamzu;
mod logitech;
mod logitech_g435_0adf;
mod machenike;
mod maono;
mod mchose;
mod mchose_a5;
mod mchose_g3;
mod mchose_headset;
mod mchose_keyboard;
mod mchose_modern;
mod misc_mice;
mod moondrop;
mod msi;
mod niceboy;
mod ninjutso;
mod nothing_ear;
mod nuphy;
mod nzxt;
mod oem_frames;
mod phonelink;
mod plantronics;
mod playstation;
mod pressplay_icarus;
mod pulsar;
mod raigor;
mod rapoo;
mod rawm;
mod razer;
mod razer_headset;
mod redragon_evision;
mod rog;
mod rongyuan;
mod royal_kludge;
mod scyrox;
mod semico;
mod sinowealth_receiver;
mod sonix;
mod sony_inzone;
mod sprime;
mod steam_controller;
mod steelseries;
mod steelseries_elite;
mod switch_pro;
mod turtlebeach;
mod vaxee;
mod vendor_frames;
mod vgn;
mod via_dongle;
mod waizowl;
mod wlmouse;
mod xinput;
mod zenith;
mod zowie;

use crate::Ctx;

pub const ORDER: &[fn(&mut Ctx)] = &[
    sinowealth_receiver::poll_sinowealth_receiver,
    glorious::poll_glorious,
    aula::poll_aula,
    aula_mouse::poll_aula_mouse,
    eweadn::poll_eweadn,
    dareu::poll_dareu,
    zenith::poll_zenith,
    darmoshark::poll_darmoshark,
    darmoshark::poll_darmoshark_nrf,
    atk_serial::poll_atk_serial,
    atk_keyboard::poll_atk_keyboard,
    redragon_evision::poll_redragon_evision,
    pulsar::poll_pulsar,
    atk_bitmouse::poll_atk_bitmouse,
    chimera::poll_chimera,
    kysona::poll_kysona,
    keychron::poll_keychron,
    cherry::poll_cherry,
    nuphy::poll_nuphy,
    arbiter::poll_arbiter,
    lamzu::poll_lamzu,
    endgame::poll_endgame,
    endgame_we::poll_endgame_we,
    vaxee::poll_vaxee,
    wlmouse::poll_wlmouse,
    finalmouse::poll_finalmouse,
    scyrox::poll_scyrox,
    ninjutso::poll_ninjutso,
    waizowl::poll_waizowl,
    sprime::poll_sprime,
    rawm::poll_rawm,
    akko::poll_akko,
    rongyuan::poll_rongyuan,
    sonix::poll_sonix,
    via_dongle::poll_via_dongle,
    semico::poll_semico,
    ajazz_ak992::poll_ajazz_ak992,
    sony_inzone::poll_sony_inzone,
    fractal::poll_fractal,
    jabra::poll_jabra,
    jbl::poll_jbl,
    rog::poll_rog,
    zowie::poll_zowie,
    royal_kludge::poll_royal_kludge,
    mchose::poll_mchose,
    mchose_a5::poll_mchose_a5,
    rapoo::poll_rapoo,
    areson::poll_areson,
    mchose_g3::poll_mchose_g3,
    msi::poll_msi,
    machenike::poll_machenike,
    niceboy::poll_niceboy,
    mchose_modern::poll_mchose_modern,
    mchose_headset::poll_mchose_headset,
    vgn::poll_vgn,
    mchose_keyboard::poll_mchose_keyboard,
    misc_mice::poll_misc_mice,
    fantech::poll_fantech,
    raigor::poll_raigor,
    razer_headset::poll_razer_headsets,
    barracuda_pro::poll_barracuda_pro,
    hyperx::poll_hyper_x,
    logitech_g435_0adf::poll_logitech_g435,
    astro::poll_astro,
    logitech::poll_logitech,
    razer::poll_razer,
    steelseries_elite::poll_steel_series_elite,
    steelseries::poll_steel_series,
    audeze::poll_audeze,
    endorfy::poll_endorfy,
    turtlebeach::poll_turtle_beach,
    akg::poll_akg,
    nzxt::poll_nzxt,
    plantronics::poll_plantronics,
    corsair::poll_corsair,
    coolermaster::poll_cooler_master,
    playstation::poll_play_station,
    switch_pro::poll_switch_pro,
    steam_controller::poll_steam_controller,
    gamesir::poll_game_sir,
    flydigi::poll_flydigi,
    gamesir::poll_game_sir_cyclone,
    gamesir_sapjoy::poll_game_sir_sapjoy,
    gamesir_ble::poll_game_sir_ble,
    eightbitdo::poll_8_bit_do,
    beitong::poll_beitong,
    airpods::poll_air_pods,
    phonelink::poll_phone_link,
    galaxy_buds::poll_galaxy_buds,
    nothing_ear::poll_nothing_ear,
    moondrop::poll_moondrop,
    darkrim::poll_darkrim,
    alienware::poll_alienware,
    maono::poll_maono,
    pressplay_icarus::poll_press_play_icarus,
    generic::poll_bluetooth,
    generic::poll_generic_hid,
    xinput::poll_x_input,
];
