/* Phase 7 M10e oracle TU for Quake/menu.c.
 *
 * WHY THIS FILE COMPOSES menu.c INSTEAD OF build.rs LISTING IT IN C_SOURCES
 *
 * The prelude's c_ref_* renames are translation-unit-wide by construction:
 * one #define rewrites the definition in the oracle source AND every call in
 * every other oracle source. For menu.c that is wrong twice over.
 *
 *   1. Six plain M_* names are already defined in this link as link doubles,
 *      because the modules that were ported before the menu call them:
 *      stubs/console_ref.c:723 (M_Menu_Main_f), stubs/draw_ref.c:243 and :250
 *      (M_DrawPic, M_Print), stubs/host_ref.c:329 (M_Init),
 *      stubs/keys_ref.c:618-640 (M_Keydown, M_Charinput, M_TextEntry,
 *      M_WaitingForKeyBinding, M_ToggleMenu_f) and stubs/stubs.c:8184
 *      (M_Menu_Quit_f). Those doubles are what console.c, draw, host.c and
 *      keys.c are held to by ~1200 existing assertions, so they stay exactly
 *      as they are -- which means THIS FILE EXPORTS NO PLAIN M_* NAME AT ALL.
 *      The oracle half is driven through c_ref_M_* and the port half through
 *      quake_rs_menu_* directly, and Quake/menu_glue.c's plain-name layer is
 *      verified by the engine link, not here.
 *
 *   2. menu.c's ~80 file statics -- every cursor, every scroll offset, the
 *      savegame table, the map/skill/lanconfig scratch -- are precisely what
 *      the two sides must NOT share: each half has to keep its own, so that a
 *      divergence in one key handler shows up in every later draw.
 *
 * So the rename layer for menu.c lives HERE, in menu.c's own TU, where it
 * renames menu.c's definitions and menu.c's internal calls and nothing else.
 *
 * WHAT IS SHARED AND WHAT IS PER SIDE
 *
 * Per side, because the prelude or the block below renamed it and the two
 * halves own disjoint copies: everything menu.c defines; the nineteen
 * renderer cvars menu.c reads (see the cvar note below); cl / cls
 * (c_ref_prelude.h:1767, :1806); sv / svs (:1145-1146), whose plain copies are
 * quake-capi's own (quake-capi/src/sv_main.rs:91, :98); hostcache /
 * hostCacheCount / net_hostport / ipv4Available / ipv6Available (:116-176);
 * the command buffer behind cmd_text (:330); the
 * cvars the already-ported client and sound strata own, which the prelude
 * renames for the oracle and quake-capi exports for the port; and the command
 * argument vector behind Cmd_Argv -- hence ctest_menu_tokenize ().
 *
 * Shared, and therefore re-seeded by the test before each side runs: the draw
 * doubles and the draw log in stubs/draw_ref.c, the console capture in
 * stubs/stubs.c, vid, glwidth, glheight, key_dest, keydown, realtime, the
 * host_cmd.c file lists (extralevels_sorted / modlist) behind the four
 * accessors below, the mouse position the SDL_GetMouseState double reports,
 * and vulkan_globals.
 *
 * THE NINETEEN RENDERER CVARS
 *
 * menu.c reads r_drawviewmodel, r_enhancedmodels, r_oit, r_particles,
 * r_rtshadows, r_scale, r_waterwarp, scr_conscale, scr_fov, scr_menuscale,
 * scr_relativescale, scr_showfps, vid_anisotropic, vid_contrast, vid_filter,
 * vid_fsaa, vid_fsaamode, vid_gamma and vid_palettize by C name and writes
 * them back by cvar name. Every one is defined in a gl_*.c the harness does
 * not compile, so this file has to define them -- and it defines them TWICE,
 * renamed for the oracle and plain for the port, because the two halves write
 * through two different cvar registries (c_ref_Cvar_Set reaches cvar.c, the
 * port's Menu_Glue_CvarSet reaches quake-capi's). ctest_menu_register_cvars ()
 * registers each copy in its own registry with the engine's own default
 * string, so a menu slider actually round-trips through a real Cvar_Set on
 * both sides instead of both sides agreeing on a "variable not found" warning.
 *
 * ADR-009. menu.c names no longjmp-capable function itself; its raise surface
 * is entirely indirect (the audit is in quake-capi/src/menu.rs's module doc).
 * The four entry points that reach it -- M_ToggleMenu_f, M_UpdateMouse,
 * M_Draw and M_Keydown -- are dispatched through ctest_try_host below, so a
 * raise is a comparable result instead of an escape past a Rust frame.
 *
 * COST, stated so it is not discovered later:
 * scripts/harness/check_ctest_symbols.sh reads C_SOURCES out of build.rs, so
 * it does not inspect this object; build.rs watches Quake/menu.c explicitly
 * instead. A missed rename below shows up only as a duplicate-symbol link
 * error, so the block is kept in step with menu.c by hand.
 */

#include "quakedef.h"

/* ---- menu.c rename block -------------------------------------------------
 * Every file-scope symbol Quake/menu.c defines. The statics do not collide,
 * but they are renamed with the rest so the block can be audited against one
 * grep of menu.c instead of against two lists.
 */

/* file-scope objects (menu.c:26-4466) */
#define vid_menucmdfn c_ref_vid_menucmdfn
#define vid_menukeyfn c_ref_vid_menukeyfn
#define m_state c_ref_m_state
#define m_entersound c_ref_m_entersound
#define m_recursiveDraw c_ref_m_recursiveDraw
#define m_is_quitting c_ref_m_is_quitting
#define m_return_state c_ref_m_return_state
#define m_return_onerror c_ref_m_return_onerror
#define m_return_reason c_ref_m_return_reason
#define m_main_cursor c_ref_m_main_cursor
#define m_mouse_moved c_ref_m_mouse_moved
#define menu_changed c_ref_menu_changed
#define m_mouse_x c_ref_m_mouse_x
#define m_mouse_y c_ref_m_mouse_y
#define m_mouse_x_pixels c_ref_m_mouse_x_pixels
#define m_mouse_y_pixels c_ref_m_mouse_y_pixels
#define scrollbar_x c_ref_scrollbar_x
#define scrollbar_y c_ref_scrollbar_y
#define scrollbar_size c_ref_scrollbar_size
#define slider_grab c_ref_slider_grab
#define scrollbar_grab c_ref_scrollbar_grab
#define crosshair_defs c_ref_crosshair_defs
#define num_crosshair_defs c_ref_num_crosshair_defs
#define m_save_demonum c_ref_m_save_demonum
#define m_singleplayer_cursor c_ref_m_singleplayer_cursor
#define m_singleplayer_showlevels c_ref_m_singleplayer_showlevels
#define m_filenames c_ref_m_filenames
#define loadable c_ref_loadable
#define load_cursor c_ref_load_cursor
#define m_multiplayer_cursor c_ref_m_multiplayer_cursor
#define setup_cursor c_ref_setup_cursor
#define setup_cursor_table c_ref_setup_cursor_table
#define setup_hostname c_ref_setup_hostname
#define setup_myname c_ref_setup_myname
#define setup_oldtop c_ref_setup_oldtop
#define setup_oldbottom c_ref_setup_oldbottom
#define setup_top c_ref_setup_top
#define setup_bottom c_ref_setup_bottom
#define m_net_cursor c_ref_m_net_cursor
#define m_first_net_item c_ref_m_first_net_item
#define m_net_items c_ref_m_net_items
#define net_helpMessage c_ref_net_helpMessage
#define game_options_cursor c_ref_game_options_cursor
#define first_game_option c_ref_first_game_option
#define graphics_options_cursor c_ref_graphics_options_cursor
#define sound_options_cursor c_ref_sound_options_cursor
#define options_cursor c_ref_options_cursor
#define bindnames c_ref_bindnames
#define keys_cursor c_ref_keys_cursor
#define bind_grab c_ref_bind_grab
#define first_key c_ref_first_key
#define help_page c_ref_help_page
#define num_mods c_ref_num_mods
#define first_mod c_ref_first_mod
#define mods_cursor c_ref_mods_cursor
#define mod_loaded_from_menu c_ref_mod_loaded_from_menu
#define m_skill_cursor c_ref_m_skill_cursor
#define m_skill_usegfx c_ref_m_skill_usegfx
#define m_skill_usecustomtitle c_ref_m_skill_usecustomtitle
#define m_skill_mapname c_ref_m_skill_mapname
#define m_skill_maptitle c_ref_m_skill_maptitle
#define m_skill_ticker c_ref_m_skill_ticker
#define msg_number c_ref_msg_number
#define was_in_menus c_ref_was_in_menus
#define lan_config_cursor c_ref_lan_config_cursor
#define lan_config_port c_ref_lan_config_port
#define lan_config_portname c_ref_lan_config_portname
#define lan_config_joinname c_ref_lan_config_joinname
#define levels c_ref_levels
#define hipnoticlevels c_ref_hipnoticlevels
#define roguelevels c_ref_roguelevels
#define episodes c_ref_episodes
#define hipnoticepisodes c_ref_hipnoticepisodes
#define rogueepisodes c_ref_rogueepisodes
#define startepisode c_ref_startepisode
#define startlevel c_ref_startlevel
#define maxplayers c_ref_maxplayers
#define mpgameoptions_cursor_table c_ref_mpgameoptions_cursor_table
#define mpgameoptions_cursor c_ref_mpgameoptions_cursor
#define search_complete c_ref_search_complete
#define search_complete_time c_ref_search_complete_time
#define slist_cursor c_ref_slist_cursor
#define slist_first c_ref_slist_first
#define slist_sorted c_ref_slist_sorted

/* functions (menu.c:165-4943) */
#define M_GetCrosshairDef c_ref_M_GetCrosshairDef
#define M_GetScale c_ref_M_GetScale
#define M_PixelToMenuCanvasCoord c_ref_M_PixelToMenuCanvasCoord
#define M_PrintHighlighted c_ref_M_PrintHighlighted
#define M_Print c_ref_M_Print
#define M_PrintElided c_ref_M_PrintElided
#define M_PrintWhite c_ref_M_PrintWhite
#define M_DrawTransPic c_ref_M_DrawTransPic
#define M_DrawPic c_ref_M_DrawPic
#define M_DrawTransPicTranslate c_ref_M_DrawTransPicTranslate
#define M_DrawTextBox c_ref_M_DrawTextBox
#define M_MenuChanged c_ref_M_MenuChanged
#define M_DrawSlider c_ref_M_DrawSlider
#define M_GetSliderPos c_ref_M_GetSliderPos
#define M_DrawScrollbar c_ref_M_DrawScrollbar
#define M_DrawCheckbox c_ref_M_DrawCheckbox
#define M_ToggleMenu_f c_ref_M_ToggleMenu_f
#define M_InScrollbar c_ref_M_InScrollbar
#define M_HandleScrollBarKeys c_ref_M_HandleScrollBarKeys
#define M_Mouse_UpdateListCursor c_ref_M_Mouse_UpdateListCursor
#define M_Mouse_UpdateCursor c_ref_M_Mouse_UpdateCursor
#define M_Menu_Main_f c_ref_M_Menu_Main_f
#define Get_Menu2 c_ref_Get_Menu2
#define M_Main_Draw c_ref_M_Main_Draw
#define M_Main_Key c_ref_M_Main_Key
#define M_Menu_SinglePlayer_f c_ref_M_Menu_SinglePlayer_f
#define M_SinglePlayer_Draw c_ref_M_SinglePlayer_Draw
#define M_SinglePlayer_Key c_ref_M_SinglePlayer_Key
#define M_ScanSaves c_ref_M_ScanSaves
#define M_Menu_Load_f c_ref_M_Menu_Load_f
#define M_Menu_Save_f c_ref_M_Menu_Save_f
#define M_Load_Draw c_ref_M_Load_Draw
#define M_Save_Draw c_ref_M_Save_Draw
#define M_Load_Key c_ref_M_Load_Key
#define M_Save_Key c_ref_M_Save_Key
#define M_Menu_MultiPlayer_f c_ref_M_Menu_MultiPlayer_f
#define M_MultiPlayer_Draw c_ref_M_MultiPlayer_Draw
#define M_MultiPlayer_Key c_ref_M_MultiPlayer_Key
#define M_Menu_Setup_f c_ref_M_Menu_Setup_f
#define M_Setup_Draw c_ref_M_Setup_Draw
#define M_Setup_Key c_ref_M_Setup_Key
#define M_Setup_Char c_ref_M_Setup_Char
#define M_Setup_TextEntry c_ref_M_Setup_TextEntry
#define M_Menu_Net_f c_ref_M_Menu_Net_f
#define M_Net_Draw c_ref_M_Net_Draw
#define M_Net_Key c_ref_M_Net_Key
#define M_Menu_GameOptions_f c_ref_M_Menu_GameOptions_f
#define M_GameOptions_AdjustSliders c_ref_M_GameOptions_AdjustSliders
#define M_GameOptions_Key c_ref_M_GameOptions_Key
#define M_GameOptions_Draw c_ref_M_GameOptions_Draw
#define M_GraphicsOptions_NumItems c_ref_M_GraphicsOptions_NumItems
#define M_Menu_GraphicsOptions_f c_ref_M_Menu_GraphicsOptions_f
#define M_GraphicsOptions_ChooseNextAASamples c_ref_M_GraphicsOptions_ChooseNextAASamples
#define M_GraphicsOptions_ChooseNextRenderScale c_ref_M_GraphicsOptions_ChooseNextRenderScale
#define M_GraphicsOptions_ChooseNextParticles c_ref_M_GraphicsOptions_ChooseNextParticles
#define M_GraphicsOptions_AdjustSliders c_ref_M_GraphicsOptions_AdjustSliders
#define M_GraphicsOptions_Key c_ref_M_GraphicsOptions_Key
#define M_GraphicsOptions_Draw c_ref_M_GraphicsOptions_Draw
#define M_Menu_SoundOptions_f c_ref_M_Menu_SoundOptions_f
#define M_SoundOptions_AdjustSliders c_ref_M_SoundOptions_AdjustSliders
#define M_SoundOptions_Key c_ref_M_SoundOptions_Key
#define M_SoundOptions_Draw c_ref_M_SoundOptions_Draw
#define M_Menu_Options_f c_ref_M_Menu_Options_f
#define M_Options_Draw c_ref_M_Options_Draw
#define M_Options_Key c_ref_M_Options_Key
#define M_Menu_Keys_f c_ref_M_Menu_Keys_f
#define M_FindKeysForCommand c_ref_M_FindKeysForCommand
#define M_UnbindCommand c_ref_M_UnbindCommand
#define M_Keys_Draw c_ref_M_Keys_Draw
#define M_Keys_Key c_ref_M_Keys_Key
#define M_Menu_Help_f c_ref_M_Menu_Help_f
#define M_Help_Draw c_ref_M_Help_Draw
#define M_Help_Key c_ref_M_Help_Key
#define M_Menu_Mods_f c_ref_M_Menu_Mods_f
#define M_Mods_Draw c_ref_M_Mods_Draw
#define M_Mods_Key c_ref_M_Mods_Key
#define M_Ticker_Init c_ref_M_Ticker_Init
#define M_Ticker_Update c_ref_M_Ticker_Update
#define M_Ticker_Key c_ref_M_Ticker_Key
#define M_PrintScroll c_ref_M_PrintScroll
#define M_DrawQuakeBar c_ref_M_DrawQuakeBar
#define M_DrawEllipsisBar c_ref_M_DrawEllipsisBar
#define M_Maps_GetMessage c_ref_M_Maps_GetMessage
#define M_Maps_IsActive c_ref_M_Maps_IsActive
#define M_Maps_AddDecoration c_ref_M_Maps_AddDecoration
#define M_Maps_AddSeparator c_ref_M_Maps_AddSeparator
#define M_Maps_IsSelectable c_ref_M_Maps_IsSelectable
#define M_Maps_Match c_ref_M_Maps_Match
#define M_Maps_ClearSearch c_ref_M_Maps_ClearSearch
#define M_Maps_GetOverflow c_ref_M_Maps_GetOverflow
#define M_Maps_ClampScroll c_ref_M_Maps_ClampScroll
#define M_Maps_AutoScroll c_ref_M_Maps_AutoScroll
#define M_Maps_CenterCursor c_ref_M_Maps_CenterCursor
#define M_Maps_SelectNextMatch c_ref_M_Maps_SelectNextMatch
#define M_Maps_SelectNextSearchMatch c_ref_M_Maps_SelectNextSearchMatch
#define M_Maps_SelectNextActive c_ref_M_Maps_SelectNextActive
#define M_Maps_UpdateMouseSelection c_ref_M_Maps_UpdateMouseSelection
#define M_Maps_Init c_ref_M_Maps_Init
#define M_Menu_Maps_f c_ref_M_Menu_Maps_f
#define M_Menu_Maps_Cmd_f c_ref_M_Menu_Maps_Cmd_f
#define M_Maps_UpdateMouse c_ref_M_Maps_UpdateMouse
#define M_Maps_Draw c_ref_M_Maps_Draw
#define M_Maps_ListKey c_ref_M_Maps_ListKey
#define M_Maps_Key c_ref_M_Maps_Key
#define M_Maps_Char c_ref_M_Maps_Char
#define M_Maps_TextEntry c_ref_M_Maps_TextEntry
#define M_SetSkillMenuMap c_ref_M_SetSkillMenuMap
#define M_Menu_Skill_f c_ref_M_Menu_Skill_f
#define M_Skill_Draw c_ref_M_Skill_Draw
#define M_Skill_Key c_ref_M_Skill_Key
#define M_Menu_Quit_f c_ref_M_Menu_Quit_f
#define M_Quit_Key c_ref_M_Quit_Key
#define M_Quit_Char c_ref_M_Quit_Char
#define M_Quit_TextEntry c_ref_M_Quit_TextEntry
#define M_Quit_Draw c_ref_M_Quit_Draw
#define M_Menu_LanConfig_f c_ref_M_Menu_LanConfig_f
#define M_LanConfig_Draw c_ref_M_LanConfig_Draw
#define validate_LanConfig c_ref_validate_LanConfig
#define M_LanConfig_Key c_ref_M_LanConfig_Key
#define M_LanConfig_Char c_ref_M_LanConfig_Char
#define M_LanConfig_TextEntry c_ref_M_LanConfig_TextEntry
#define M_Menu_MPGameOptions_f c_ref_M_Menu_MPGameOptions_f
#define M_MPGameOptions_Draw c_ref_M_MPGameOptions_Draw
#define M_NetStart_Change c_ref_M_NetStart_Change
#define M_MPGameOptions_Key c_ref_M_MPGameOptions_Key
#define M_Menu_Search_f c_ref_M_Menu_Search_f
#define M_Search_Draw c_ref_M_Search_Draw
#define M_Menu_ServerList_f c_ref_M_Menu_ServerList_f
#define M_ServerList_Draw c_ref_M_ServerList_Draw
#define M_ServerList_Key c_ref_M_ServerList_Key
#define M_CheckCustomGfx c_ref_M_CheckCustomGfx
#define M_CheckMods c_ref_M_CheckMods
#define M_Init c_ref_M_Init
#define M_NewGame c_ref_M_NewGame
#define M_UpdateMouse c_ref_M_UpdateMouse
#define M_Draw c_ref_M_Draw
#define M_Keydown c_ref_M_Keydown
#define M_Charinput c_ref_M_Charinput
#define M_TextEntry c_ref_M_TextEntry
#define M_WaitingForKeyBinding c_ref_M_WaitingForKeyBinding
#define M_ConfigureNetSubsystem c_ref_M_ConfigureNetSubsystem

/* The renderer cvars menu.c reads by C name. No gl_*.c is compiled here, so
 * both copies are defined below; see the header. */

#define r_drawviewmodel c_ref_r_drawviewmodel
#define r_enhancedmodels c_ref_r_enhancedmodels
#define r_oit c_ref_r_oit
#define r_particles c_ref_r_particles
#define r_rtshadows c_ref_r_rtshadows
#define r_scale c_ref_r_scale
#define r_waterwarp c_ref_r_waterwarp
#define scr_conscale c_ref_scr_conscale
#define scr_fov c_ref_scr_fov
#define scr_menuscale c_ref_scr_menuscale
#define scr_relativescale c_ref_scr_relativescale
#define scr_showfps c_ref_scr_showfps
#define vid_anisotropic c_ref_vid_anisotropic
#define vid_contrast c_ref_vid_contrast
#define vid_filter c_ref_vid_filter
#define vid_fsaa c_ref_vid_fsaa
#define vid_fsaamode c_ref_vid_fsaamode
#define vid_gamma c_ref_vid_gamma
#define vid_palettize c_ref_vid_palettize

/* menu.h is not force-included by the prelude (it declares M_Print / M_Draw
 * over the Vulkan-typed cb_context_t), and quakedef.h is a no-op here, so
 * menu.c would see neither enum m_state_e nor crosshair_t. Including it after
 * the block above rewrites its declarations too, which is what keeps the
 * oracle build warning-clean. */
/* Sixteen cvars menu.c writes that no other rename layer split. Left shared
 * they would be one object both halves write through, and -- worse -- one
 * object that can be registered in only ONE of the two cvar registries,
 * because a cvar_t registered twice corrupts both ->next chains
 * (stubs/stubs.c:1981 records that the hard way). Unregistered, every
 * Cvar_Set the options and multiplayer menus perform would be a no-op that
 * agrees with itself: symmetric, and worth nothing. Split here, each half
 * registers its own copy in its own registry and the writes are real. */
#define scr_style             c_ref_scr_style
#define autoload              c_ref_autoload
#define autofastload          c_ref_autofastload
#define r_lerpmodels          c_ref_r_lerpmodels
#define r_lerpmove            c_ref_r_lerpmove
#define r_lerpturn            c_ref_r_lerpturn
#define host_maxfps           c_ref_host_maxfps
#define hostname              c_ref_hostname
#define scr_sbaralpha         c_ref_scr_sbaralpha
#define scr_viewsize          c_ref_scr_viewsize
#define bgm_extmusic          c_ref_bgm_extmusic
#define coop                  c_ref_coop
#define teamplay              c_ref_teamplay
#define skill                 c_ref_skill
#define fraglimit             c_ref_fraglimit
#define timelimit             c_ref_timelimit

#include "menu.h"

/* The oracle's nineteen renderer cvars. Declared here rather than left to the
 * definitions below because menu.c reads them before this file defines them. */
extern cvar_t c_ref_r_drawviewmodel;
extern cvar_t c_ref_r_enhancedmodels;
extern cvar_t c_ref_r_oit;
extern cvar_t c_ref_r_particles;
extern cvar_t c_ref_r_rtshadows;
extern cvar_t c_ref_r_scale;
extern cvar_t c_ref_r_waterwarp;
extern cvar_t c_ref_scr_conscale;
extern cvar_t c_ref_scr_fov;
extern cvar_t c_ref_scr_menuscale;
extern cvar_t c_ref_scr_relativescale;
extern cvar_t c_ref_scr_showfps;
extern cvar_t c_ref_vid_anisotropic;
extern cvar_t c_ref_vid_contrast;
extern cvar_t c_ref_vid_filter;
extern cvar_t c_ref_vid_fsaa;
extern cvar_t c_ref_vid_fsaamode;
extern cvar_t c_ref_vid_gamma;
extern cvar_t c_ref_vid_palettize;

/* The sixteen split above. The seven host.c already owns are declared; the
 * nine with no c_ref_ definition anywhere in the link are defined here with
 * the engine's own name, default and flags. */
cvar_t c_ref_scr_style = {"scr_style", "0", CVAR_ARCHIVE};
extern cvar_t c_ref_autoload;
extern cvar_t c_ref_autofastload;
cvar_t c_ref_r_lerpmodels = {"r_lerpmodels", "1", CVAR_ARCHIVE};
cvar_t c_ref_r_lerpmove = {"r_lerpmove", "1", CVAR_ARCHIVE};
cvar_t c_ref_r_lerpturn = {"r_lerpturn", "1", CVAR_ARCHIVE};
extern cvar_t c_ref_host_maxfps;
cvar_t c_ref_hostname = {"hostname", "UNNAMED", CVAR_SERVERINFO};
cvar_t c_ref_scr_sbaralpha = {"scr_sbaralpha", "0.75", CVAR_ARCHIVE};
cvar_t c_ref_scr_viewsize = {"viewsize", "100", CVAR_ARCHIVE};
cvar_t c_ref_bgm_extmusic = {"bgm_extmusic", "1", CVAR_ARCHIVE};
cvar_t c_ref_coop = {"coop", "0", CVAR_NONE};
extern cvar_t c_ref_teamplay;
extern cvar_t c_ref_skill;
extern cvar_t c_ref_fraglimit;
extern cvar_t c_ref_timelimit;

#include "menu.c"

/* =========================================================================
 * THE PLAIN HALF -- the ctest-link mirror of Quake/menu_glue.c, minus every
 * plain M_* name (see point 1 in the header).
 * ========================================================================= */

#undef vid_menucmdfn
#undef vid_menukeyfn
#undef m_state
#undef m_entersound
#undef m_recursiveDraw
#undef m_is_quitting
#undef m_return_state
#undef m_return_onerror
#undef m_return_reason
#undef m_main_cursor
#undef m_mouse_moved
#undef menu_changed
#undef m_mouse_x
#undef m_mouse_y
#undef m_mouse_x_pixels
#undef m_mouse_y_pixels
#undef scrollbar_x
#undef scrollbar_y
#undef scrollbar_size
#undef slider_grab
#undef scrollbar_grab
#undef crosshair_defs
#undef num_crosshair_defs
#undef m_save_demonum
#undef m_singleplayer_cursor
#undef m_singleplayer_showlevels
#undef m_filenames
#undef loadable
#undef load_cursor
#undef m_multiplayer_cursor
#undef setup_cursor
#undef setup_cursor_table
#undef setup_hostname
#undef setup_myname
#undef setup_oldtop
#undef setup_oldbottom
#undef setup_top
#undef setup_bottom
#undef m_net_cursor
#undef m_first_net_item
#undef m_net_items
#undef net_helpMessage
#undef game_options_cursor
#undef first_game_option
#undef graphics_options_cursor
#undef sound_options_cursor
#undef options_cursor
#undef bindnames
#undef keys_cursor
#undef bind_grab
#undef first_key
#undef help_page
#undef num_mods
#undef first_mod
#undef mods_cursor
#undef mod_loaded_from_menu
#undef m_skill_cursor
#undef m_skill_usegfx
#undef m_skill_usecustomtitle
#undef m_skill_mapname
#undef m_skill_maptitle
#undef m_skill_ticker
#undef msg_number
#undef was_in_menus
#undef lan_config_cursor
#undef lan_config_port
#undef lan_config_portname
#undef lan_config_joinname
#undef levels
#undef hipnoticlevels
#undef roguelevels
#undef episodes
#undef hipnoticepisodes
#undef rogueepisodes
#undef startepisode
#undef startlevel
#undef maxplayers
#undef mpgameoptions_cursor_table
#undef mpgameoptions_cursor
#undef search_complete
#undef search_complete_time
#undef slist_cursor
#undef slist_first
#undef slist_sorted
#undef M_GetCrosshairDef
#undef M_GetScale
#undef M_PixelToMenuCanvasCoord
#undef M_PrintHighlighted
#undef M_Print
#undef M_PrintElided
#undef M_PrintWhite
#undef M_DrawTransPic
#undef M_DrawPic
#undef M_DrawTransPicTranslate
#undef M_DrawTextBox
#undef M_MenuChanged
#undef M_DrawSlider
#undef M_GetSliderPos
#undef M_DrawScrollbar
#undef M_DrawCheckbox
#undef M_ToggleMenu_f
#undef M_InScrollbar
#undef M_HandleScrollBarKeys
#undef M_Mouse_UpdateListCursor
#undef M_Mouse_UpdateCursor
#undef M_Menu_Main_f
#undef Get_Menu2
#undef M_Main_Draw
#undef M_Main_Key
#undef M_Menu_SinglePlayer_f
#undef M_SinglePlayer_Draw
#undef M_SinglePlayer_Key
#undef M_ScanSaves
#undef M_Menu_Load_f
#undef M_Menu_Save_f
#undef M_Load_Draw
#undef M_Save_Draw
#undef M_Load_Key
#undef M_Save_Key
#undef M_Menu_MultiPlayer_f
#undef M_MultiPlayer_Draw
#undef M_MultiPlayer_Key
#undef M_Menu_Setup_f
#undef M_Setup_Draw
#undef M_Setup_Key
#undef M_Setup_Char
#undef M_Setup_TextEntry
#undef M_Menu_Net_f
#undef M_Net_Draw
#undef M_Net_Key
#undef M_Menu_GameOptions_f
#undef M_GameOptions_AdjustSliders
#undef M_GameOptions_Key
#undef M_GameOptions_Draw
#undef M_GraphicsOptions_NumItems
#undef M_Menu_GraphicsOptions_f
#undef M_GraphicsOptions_ChooseNextAASamples
#undef M_GraphicsOptions_ChooseNextRenderScale
#undef M_GraphicsOptions_ChooseNextParticles
#undef M_GraphicsOptions_AdjustSliders
#undef M_GraphicsOptions_Key
#undef M_GraphicsOptions_Draw
#undef M_Menu_SoundOptions_f
#undef M_SoundOptions_AdjustSliders
#undef M_SoundOptions_Key
#undef M_SoundOptions_Draw
#undef M_Menu_Options_f
#undef M_Options_Draw
#undef M_Options_Key
#undef M_Menu_Keys_f
#undef M_FindKeysForCommand
#undef M_UnbindCommand
#undef M_Keys_Draw
#undef M_Keys_Key
#undef M_Menu_Help_f
#undef M_Help_Draw
#undef M_Help_Key
#undef M_Menu_Mods_f
#undef M_Mods_Draw
#undef M_Mods_Key
#undef M_Ticker_Init
#undef M_Ticker_Update
#undef M_Ticker_Key
#undef M_PrintScroll
#undef M_DrawQuakeBar
#undef M_DrawEllipsisBar
#undef M_Maps_GetMessage
#undef M_Maps_IsActive
#undef M_Maps_AddDecoration
#undef M_Maps_AddSeparator
#undef M_Maps_IsSelectable
#undef M_Maps_Match
#undef M_Maps_ClearSearch
#undef M_Maps_GetOverflow
#undef M_Maps_ClampScroll
#undef M_Maps_AutoScroll
#undef M_Maps_CenterCursor
#undef M_Maps_SelectNextMatch
#undef M_Maps_SelectNextSearchMatch
#undef M_Maps_SelectNextActive
#undef M_Maps_UpdateMouseSelection
#undef M_Maps_Init
#undef M_Menu_Maps_f
#undef M_Menu_Maps_Cmd_f
#undef M_Maps_UpdateMouse
#undef M_Maps_Draw
#undef M_Maps_ListKey
#undef M_Maps_Key
#undef M_Maps_Char
#undef M_Maps_TextEntry
#undef M_SetSkillMenuMap
#undef M_Menu_Skill_f
#undef M_Skill_Draw
#undef M_Skill_Key
#undef M_Menu_Quit_f
#undef M_Quit_Key
#undef M_Quit_Char
#undef M_Quit_TextEntry
#undef M_Quit_Draw
#undef M_Menu_LanConfig_f
#undef M_LanConfig_Draw
#undef validate_LanConfig
#undef M_LanConfig_Key
#undef M_LanConfig_Char
#undef M_LanConfig_TextEntry
#undef M_Menu_MPGameOptions_f
#undef M_MPGameOptions_Draw
#undef M_NetStart_Change
#undef M_MPGameOptions_Key
#undef M_Menu_Search_f
#undef M_Search_Draw
#undef M_Menu_ServerList_f
#undef M_ServerList_Draw
#undef M_ServerList_Key
#undef M_CheckCustomGfx
#undef M_CheckMods
#undef M_Init
#undef M_NewGame
#undef M_UpdateMouse
#undef M_Draw
#undef M_Keydown
#undef M_Charinput
#undef M_TextEntry
#undef M_WaitingForKeyBinding
#undef M_ConfigureNetSubsystem
#undef r_drawviewmodel
#undef r_enhancedmodels
#undef r_oit
#undef r_particles
#undef r_rtshadows
#undef r_scale
#undef r_waterwarp
#undef scr_conscale
#undef scr_fov
#undef scr_menuscale
#undef scr_relativescale
#undef scr_showfps
#undef vid_anisotropic
#undef vid_contrast
#undef vid_filter
#undef vid_fsaa
#undef vid_fsaamode
#undef vid_gamma
#undef vid_palettize
#undef cl
#undef cls

extern client_state_t  cl;	/* quake-capi's cl_main port owns it (ADR-007) */
extern client_static_t cls; /* likewise */

/* ---------------------------------------------------------------------------
 * Plain-name escape hatch. c_ref_prelude.h rewrites these five names for the
 * whole translation unit (lines 316-321, 354), which is what the oracle half
 * wants and the port half must not have: Quake/menu_glue.c's shims reach
 * quake-capi's cvar registry, not cvar.c's, and the fixtures below have to
 * tokenize into both argument vectors. Same pattern as stubs/sbar_ref.c:556
 * and stubs/console_ref.c:1678.
 */

#undef Cvar_Set
#undef Cvar_SetValue
#undef Cvar_SetValueQuick
#undef Cvar_RegisterVariable
#undef Cmd_TokenizeString

void Cvar_Set (const char *var_name, const char *value);
void Cvar_SetValue (const char *var_name, float value);
void Cvar_SetValueQuick (cvar_t *var, float value);
void Cvar_RegisterVariable (cvar_t *variable);
void Cmd_TokenizeString (const char *text);

extern int		   ctest_try_host (void (*fn) (void *), void *arg);
extern void		   ctest_draw_record (const char *fmt, ...);
extern qpic_t	  *ctest_draw_pic (const char *name);

/* The port half's copies of the eight glue-owned objects. Five already exist
 * in this link (stubs/stubs.c:3103-3106, stubs/keys_ref.c:690); m_entersound
 * is defined just below. */
extern int		m_state;
extern int		m_return_state;
extern qboolean m_is_quitting;
extern qboolean m_return_onerror;
extern char		m_return_reason[32];

/* ---------------------------------------------------------------------------
 * Glue-owned storage (menu_glue.c:74-89). Four of the eight objects are
 * already defined for the port half at stubs.c:3103-3106 and a fifth
 * (m_is_quitting) at keys_ref.c:690; vid_menucmdfn and vid_menukeyfn are
 * defined by menu.c and read by nothing in this link, so the oracle's
 * renamed copies are the only ones needed. That leaves m_entersound.
 */

qboolean m_entersound; /* menu.c:85 */

/* ---------------------------------------------------------------------------
 * The nineteen renderer cvars, twice -- see THE NINETEEN RENDERER CVARS in
 * the header. The default strings are copied from the gl_*.c that registers
 * each one in the shipping build.
 */

cvar_t c_ref_r_drawviewmodel = {"r_drawviewmodel", "1", CVAR_NONE};
cvar_t r_drawviewmodel = {"r_drawviewmodel", "1", CVAR_NONE};
cvar_t c_ref_r_enhancedmodels = {"r_enhancedmodels", "1", CVAR_ARCHIVE};
cvar_t r_enhancedmodels = {"r_enhancedmodels", "1", CVAR_ARCHIVE};
cvar_t c_ref_r_oit = {"r_oit", "1", CVAR_ARCHIVE};
cvar_t r_oit = {"r_oit", "1", CVAR_ARCHIVE};
cvar_t c_ref_r_particles = {"r_particles", "1", CVAR_ARCHIVE};
cvar_t r_particles = {"r_particles", "1", CVAR_ARCHIVE};
cvar_t c_ref_r_rtshadows = {"r_rtshadows", "2", CVAR_ARCHIVE};
cvar_t r_rtshadows = {"r_rtshadows", "2", CVAR_ARCHIVE};
cvar_t c_ref_r_scale = {"r_scale", "1", CVAR_ARCHIVE};
cvar_t r_scale = {"r_scale", "1", CVAR_ARCHIVE};
cvar_t c_ref_r_waterwarp = {"r_waterwarp", "1", CVAR_ARCHIVE};
cvar_t r_waterwarp = {"r_waterwarp", "1", CVAR_ARCHIVE};
cvar_t c_ref_scr_conscale = {"scr_conscale", "1", CVAR_ARCHIVE};
cvar_t scr_conscale = {"scr_conscale", "1", CVAR_ARCHIVE};
cvar_t c_ref_scr_fov = {"fov", "90", CVAR_ARCHIVE};
cvar_t scr_fov = {"fov", "90", CVAR_ARCHIVE};
cvar_t c_ref_scr_menuscale = {"scr_menuscale", "1", CVAR_ARCHIVE};
cvar_t scr_menuscale = {"scr_menuscale", "1", CVAR_ARCHIVE};
cvar_t c_ref_scr_relativescale = {"scr_relativescale", "2", CVAR_ARCHIVE};
cvar_t scr_relativescale = {"scr_relativescale", "2", CVAR_ARCHIVE};
cvar_t c_ref_scr_showfps = {"scr_showfps", "0", CVAR_ARCHIVE};
cvar_t scr_showfps = {"scr_showfps", "0", CVAR_ARCHIVE};
cvar_t c_ref_vid_anisotropic = {"vid_anisotropic", "0", CVAR_ARCHIVE};
cvar_t vid_anisotropic = {"vid_anisotropic", "0", CVAR_ARCHIVE};
cvar_t c_ref_vid_contrast = {"contrast", "1.4", CVAR_ARCHIVE};
cvar_t vid_contrast = {"contrast", "1.4", CVAR_ARCHIVE};
cvar_t c_ref_vid_filter = {"vid_filter", "0", CVAR_ARCHIVE};
cvar_t vid_filter = {"vid_filter", "0", CVAR_ARCHIVE};
cvar_t c_ref_vid_fsaa = {"vid_fsaa", "0", CVAR_ARCHIVE};
cvar_t vid_fsaa = {"vid_fsaa", "0", CVAR_ARCHIVE};
cvar_t c_ref_vid_fsaamode = {"vid_fsaamode", "0", CVAR_ARCHIVE};
cvar_t vid_fsaamode = {"vid_fsaamode", "0", CVAR_ARCHIVE};
cvar_t c_ref_vid_gamma = {"gamma", "0.9", CVAR_ARCHIVE};
cvar_t vid_gamma = {"gamma", "0.9", CVAR_ARCHIVE};
cvar_t c_ref_vid_palettize = {"vid_palettize", "0", CVAR_ARCHIVE};
cvar_t vid_palettize = {"vid_palettize", "0", CVAR_ARCHIVE};

/* The seventeen menu cvars c_ref_prelude.h renames: the oracle half reads the
 * c_ref_ object its owning oracle source defines and the port half reads
 * quake-capi's, so the pair table below needs both spellings in scope. */
#undef cl_confirmquit
#undef snd_waterfx
#undef cl_bob
#undef cl_rollangle
#undef v_gunkick
#undef crosshair
#undef crosshair_def
#undef cl_name
#undef cl_topcolor
#undef cl_bottomcolor
#undef sensitivity
#undef m_pitch
#undef cl_alwaysrun
#undef cl_forwardspeed
#undef cl_startdemos
#undef bgmvolume
#undef sfxvolume

extern cvar_t cl_confirmquit;
extern cvar_t c_ref_cl_confirmquit;
extern cvar_t snd_waterfx;
extern cvar_t c_ref_snd_waterfx;
extern cvar_t cl_bob;
extern cvar_t c_ref_cl_bob;
extern cvar_t cl_rollangle;
extern cvar_t c_ref_cl_rollangle;
extern cvar_t v_gunkick;
extern cvar_t c_ref_v_gunkick;
extern cvar_t crosshair;
extern cvar_t c_ref_crosshair;
extern cvar_t crosshair_def;
extern cvar_t c_ref_crosshair_def;
extern cvar_t cl_name;
extern cvar_t c_ref_cl_name;
extern cvar_t cl_topcolor;
extern cvar_t c_ref_cl_topcolor;
extern cvar_t cl_bottomcolor;
extern cvar_t c_ref_cl_bottomcolor;
extern cvar_t sensitivity;
extern cvar_t c_ref_sensitivity;
extern cvar_t m_pitch;
extern cvar_t c_ref_m_pitch;
extern cvar_t cl_alwaysrun;
extern cvar_t c_ref_cl_alwaysrun;
extern cvar_t cl_forwardspeed;
extern cvar_t c_ref_cl_forwardspeed;
extern cvar_t cl_startdemos;
extern cvar_t c_ref_cl_startdemos;
extern cvar_t bgmvolume;
extern cvar_t c_ref_bgmvolume;
extern cvar_t sfxvolume;
extern cvar_t c_ref_sfxvolume;

/* The sixteen this file split for itself. Inside the rename block above the
 * plain spelling still expands to the c_ref_ twin, so the port's slot in the
 * table below would silently be the oracle's object -- and registering one
 * cvar_t in both registries merges the two ->next chains. Undo the rename
 * here, exactly as the seventeen prelude-renamed ones are undone above. */
#undef scr_style
#undef autoload
#undef autofastload
#undef r_lerpmodels
#undef r_lerpmove
#undef r_lerpturn
#undef host_maxfps
#undef hostname
#undef scr_sbaralpha
#undef scr_viewsize
#undef bgm_extmusic
#undef coop
#undef teamplay
#undef skill
#undef fraglimit
#undef timelimit

extern cvar_t scr_style;
extern cvar_t autoload;
extern cvar_t autofastload;
extern cvar_t r_lerpmodels;
extern cvar_t r_lerpmove;
extern cvar_t r_lerpturn;
extern cvar_t host_maxfps;
extern cvar_t hostname;
extern cvar_t scr_sbaralpha;
extern cvar_t scr_viewsize;
extern cvar_t bgm_extmusic;
extern cvar_t coop;
extern cvar_t teamplay;
extern cvar_t skill;
extern cvar_t fraglimit;
extern cvar_t timelimit;

/* Every cvar menu.c reads or writes (quake-c-sys/src/menu.rs:444-502), paired
 * per side, each with the engine's own default string so both halves start
 * from the shipping value and ctest_menu_reset_cvars can put them back.
 * `registered` is the one entry that stays a single object: menu.c only reads
 * it (:638), it is CVAR_ROM so nothing can write it, and splitting a read-only
 * constant buys nothing. */
static struct
{
	cvar_t	   *c;
	cvar_t	   *r;
	const char *def;
} ctest_menu_cvar_table[] = {
	{&c_ref_scr_fov, &scr_fov, "90"},
	{&c_ref_scr_showfps, &scr_showfps, "0"},
	{&c_ref_cl_confirmquit, &cl_confirmquit, "0"},
	{&c_ref_scr_style, &scr_style, "0"},
	{&c_ref_scr_menuscale, &scr_menuscale, "1"},
	{&c_ref_autoload, &autoload, "1"},
	{&c_ref_autofastload, &autofastload, "0"},
	{&c_ref_r_rtshadows, &r_rtshadows, "2"},
	{&c_ref_r_particles, &r_particles, "1"},
	{&c_ref_r_oit, &r_oit, "1"},
	{&c_ref_r_enhancedmodels, &r_enhancedmodels, "1"},
	{&c_ref_r_lerpmodels, &r_lerpmodels, "1"},
	{&c_ref_r_lerpmove, &r_lerpmove, "1"},
	{&c_ref_r_lerpturn, &r_lerpturn, "1"},
	{&c_ref_vid_filter, &vid_filter, "0"},
	{&c_ref_vid_palettize, &vid_palettize, "0"},
	{&c_ref_vid_anisotropic, &vid_anisotropic, "0"},
	{&c_ref_vid_fsaa, &vid_fsaa, "0"},
	{&c_ref_vid_fsaamode, &vid_fsaamode, "0"},
	{&c_ref_host_maxfps, &host_maxfps, "200"},
	{&c_ref_snd_waterfx, &snd_waterfx, "1"},
	{&c_ref_cl_bob, &cl_bob, "0.02"},
	{&c_ref_cl_rollangle, &cl_rollangle, "2.0"},
	{&c_ref_v_gunkick, &v_gunkick, "1"},
	{&c_ref_crosshair, &crosshair, "1"},
	{&c_ref_crosshair_def, &crosshair_def, "0"},
	{&c_ref_cl_name, &cl_name, "player"},
	{&c_ref_hostname, &hostname, "UNNAMED"},
	{&c_ref_cl_topcolor, &cl_topcolor, "0"},
	{&c_ref_cl_bottomcolor, &cl_bottomcolor, "0"},
	{&c_ref_scr_relativescale, &scr_relativescale, "2"},
	{&c_ref_scr_conscale, &scr_conscale, "1"},
	{&c_ref_scr_sbaralpha, &scr_sbaralpha, "0.75"},
	{&c_ref_scr_viewsize, &scr_viewsize, "100"},
	{&c_ref_sensitivity, &sensitivity, "3"},
	{&c_ref_m_pitch, &m_pitch, "0.022"},
	{&c_ref_r_drawviewmodel, &r_drawviewmodel, "1"},
	{&c_ref_r_scale, &r_scale, "1"},
	{&c_ref_r_waterwarp, &r_waterwarp, "1"},
	{&c_ref_cl_alwaysrun, &cl_alwaysrun, "1"},
	{&c_ref_cl_forwardspeed, &cl_forwardspeed, "200"},
	{&c_ref_cl_startdemos, &cl_startdemos, "1"},
	{&c_ref_vid_gamma, &vid_gamma, "0.9"},
	{&c_ref_vid_contrast, &vid_contrast, "1.4"},
	{&c_ref_bgmvolume, &bgmvolume, "1"},
	{&c_ref_bgm_extmusic, &bgm_extmusic, "1"},
	{&c_ref_sfxvolume, &sfxvolume, "0.7"},
	{&registered, &registered, NULL}, /* read-only, see above */
	{&c_ref_coop, &coop, "0"},
	{&c_ref_teamplay, &teamplay, "0"},
	{&c_ref_skill, &skill, "1"},
	{&c_ref_fraglimit, &fraglimit, "0"},
	{&c_ref_timelimit, &timelimit, "0"},
};

#define CTEST_MENU_NUM_CVARS ((int)(sizeof (ctest_menu_cvar_table) / sizeof (ctest_menu_cvar_table[0])))

int ctest_menu_cvar_count (void)
{
	return CTEST_MENU_NUM_CVARS;
}

const char *ctest_menu_cvar_name (int idx)
{
	if (idx < 0 || idx >= CTEST_MENU_NUM_CVARS)
		return "";
	return ctest_menu_cvar_table[idx].c->name;
}

/* Only the nineteen this file defines: the other thirty-four belong to a
 * module that already registered them (or deliberately did not), and a second
 * registration would relink somebody else's cvar_t into a second registry's
 * ->next chain. */
void ctest_menu_register_cvars (void)
{
	static qboolean done;
	int				i;

	if (done)
		return;
	done = true;
	for (i = 0; i < CTEST_MENU_NUM_CVARS; i++)
	{
		if (!ctest_menu_cvar_table[i].def)
			continue;
		c_ref_Cvar_RegisterVariable (ctest_menu_cvar_table[i].c);
		Cvar_RegisterVariable (ctest_menu_cvar_table[i].r);
	}
}

#undef Cvar_FindVar
extern cvar_t *Cvar_FindVar (const char *var_name);

/* 0 = the name is absent from that side's registry, 1 = it resolves to the
 * very object menu.c reads, 2 = it resolves to a different object. */
int ctest_menu_cvar_registered (int side, int idx)
{
	cvar_t *want;
	cvar_t *got;

	if (idx < 0 || idx >= CTEST_MENU_NUM_CVARS)
		return 0;
	want = side ? ctest_menu_cvar_table[idx].c : ctest_menu_cvar_table[idx].r;
	got = side ? c_ref_Cvar_FindVar (want->name) : Cvar_FindVar (want->name);
	if (!got)
		return 0;
	return got == want ? 1 : 2;
}

void ctest_menu_reset_cvars (void)
{
	int i;

	for (i = 0; i < CTEST_MENU_NUM_CVARS; i++)
	{
		if (!ctest_menu_cvar_table[i].def)
			continue;
		c_ref_Cvar_Set (ctest_menu_cvar_table[i].c->name, ctest_menu_cvar_table[i].def);
		Cvar_Set (ctest_menu_cvar_table[i].r->name, ctest_menu_cvar_table[i].def);
	}
}

const char *ctest_menu_cvar_string (int side, int idx)
{
	if (idx < 0 || idx >= CTEST_MENU_NUM_CVARS)
		return "";
	return side ? ctest_menu_cvar_table[idx].c->string : ctest_menu_cvar_table[idx].r->string;
}

float ctest_menu_cvar_value (int side, int idx)
{
	if (idx < 0 || idx >= CTEST_MENU_NUM_CVARS)
		return 0.0f;
	return side ? ctest_menu_cvar_table[idx].c->value : ctest_menu_cvar_table[idx].r->value;
}

void ctest_menu_cvar_set (int side, int idx, const char *value)
{
	if (idx < 0 || idx >= CTEST_MENU_NUM_CVARS)
		return;
	if (side)
		c_ref_Cvar_Set (ctest_menu_cvar_table[idx].c->name, value);
	else
		Cvar_Set (ctest_menu_cvar_table[idx].r->name, value);
}

/* ---------------------------------------------------------------------------
 * The mouse position SDL_GetMouseState reports. Shared: menu.c reaches SDL
 * directly and the port reaches Menu_Glue_GetMouseState, and both have to be
 * told the same pixel position for M_UpdateMouse to be comparable.
 */

static int ctest_menu_mouse_x;
static int ctest_menu_mouse_y;

void ctest_menu_set_mouse (int x, int y)
{
	ctest_menu_mouse_x = x;
	ctest_menu_mouse_y = y;
}

/* menu.c:4632 takes this branch because the harness builds without USE_SDL3.
 * Declared in c_ref_prelude.h; no Rust translation unit names it. */
uint32_t SDL_GetMouseState (int *x, int *y)
{
	if (x)
		*x = ctest_menu_mouse_x;
	if (y)
		*y = ctest_menu_mouse_y;
	return 0;
}

/* ---------------------------------------------------------------------------
 * Quake/menu_glue.c's fifteen shims, doubled for this link.
 *
 * The ten that wrap Host_Guard in the engine wrap ctest_try_host here: it
 * traps the harness's Host_Error (stubs.c:1435) the same way Host_Guard traps
 * the engine's longjmp, and returns 0 / 1 where Host_Guard returns
 * HOST_GUARD_OK / a raise code. That keeps the port half's status plumbing
 * under test instead of stubbed out.
 *
 * Every target below is either already shared with the oracle half (the
 * console toggle, the video menu, SCR_) or is the same Rust body the oracle
 * half reaches through its own c_ref_ name (CL_NextDemo, the cvar writes).
 */

typedef struct
{
	const char *s;
	const char *t;
	float		f;
	int			i;
	int		   *outi;
	cvar_t	   *var;
	void	   *cbx;
} ctest_menu_arg_t;

static void ctest_menu_invoke_toggle_console (void *p)
{
	(void)p;
	Con_ToggleConsole_f ();
}

int Menu_Glue_ToggleConsole (void)
{
	return ctest_try_host (ctest_menu_invoke_toggle_console, NULL);
}

/* cl_main.c:302 is ported and quake-capi exports it as a status-returning
 * core, but no plain CL_NextDemo exists in this link -- c_ref_prelude.h:1788
 * renames the oracle definition and Quake/cl_main_glue.c is not compiled
 * here. The port half therefore enters the Rust core the same way
 * Quake/cl_main_glue.c does, and the oracle half keeps reaching
 * c_ref_CL_NextDemo from inside menu.c. */
extern int quake_rs_cl_next_demo (void);

static void ctest_menu_invoke_next_demo (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	a->i = quake_rs_cl_next_demo ();
}

int Menu_Glue_NextDemo (void)
{
	ctest_menu_arg_t arg = {0};

	if (ctest_try_host (ctest_menu_invoke_next_demo, &arg))
		return 1;
	return arg.i;
}

static void ctest_menu_invoke_modal_message (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	*a->outi = SCR_ModalMessage (a->s, a->f);
}

int Menu_Glue_ModalMessage (const char *text, float timeout, int *out)
{
	ctest_menu_arg_t arg = {0};

	arg.s = text;
	arg.f = timeout;
	arg.outi = out;
	return ctest_try_host (ctest_menu_invoke_modal_message, &arg);
}

static void ctest_menu_invoke_begin_loading_plaque (void *p)
{
	(void)p;
	SCR_BeginLoadingPlaque ();
}

int Menu_Glue_BeginLoadingPlaque (void)
{
	return ctest_try_host (ctest_menu_invoke_begin_loading_plaque, NULL);
}

static void ctest_menu_invoke_cvar_set (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	Cvar_Set (a->s, a->t);
}

int Menu_Glue_CvarSet (const char *name, const char *value)
{
	ctest_menu_arg_t arg = {0};

	arg.s = name;
	arg.t = value;
	return ctest_try_host (ctest_menu_invoke_cvar_set, &arg);
}

static void ctest_menu_invoke_cvar_set_value (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	Cvar_SetValue (a->s, a->f);
}

int Menu_Glue_CvarSetValue (const char *name, float value)
{
	ctest_menu_arg_t arg = {0};

	arg.s = name;
	arg.f = value;
	return ctest_try_host (ctest_menu_invoke_cvar_set_value, &arg);
}

static void ctest_menu_invoke_cvar_set_value_quick (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	Cvar_SetValueQuick (a->var, a->f);
}

int Menu_Glue_CvarSetValueQuick (cvar_t *var, float value)
{
	ctest_menu_arg_t arg = {0};

	arg.var = var;
	arg.f = value;
	return ctest_try_host (ctest_menu_invoke_cvar_set_value_quick, &arg);
}

static void ctest_menu_invoke_menu_video (void *p)
{
	(void)p;
	M_Menu_Video_f ();
}

int Menu_Glue_MenuVideo (void)
{
	return ctest_try_host (ctest_menu_invoke_menu_video, NULL);
}

static void ctest_menu_invoke_video_draw (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	M_Video_Draw ((cb_context_t *)a->cbx);
}

int Menu_Glue_VideoDraw (cb_context_t *cbx)
{
	ctest_menu_arg_t arg = {0};

	arg.cbx = cbx;
	return ctest_try_host (ctest_menu_invoke_video_draw, &arg);
}

static void ctest_menu_invoke_video_key (void *p)
{
	ctest_menu_arg_t *a = (ctest_menu_arg_t *)p;
	M_Video_Key (a->i);
}

int Menu_Glue_VideoKey (int key)
{
	ctest_menu_arg_t arg = {0};

	arg.i = key;
	return ctest_try_host (ctest_menu_invoke_video_key, &arg);
}

/* The five non-guard shims. RayQuery / SampleRateShading /
 * MaxSamplerAnisotropy read the one shared vulkan_globals
 * (stubs/pr_ext_ref.c:248) that menu.c's oracle half reads directly, so
 * ctest_menu_set_caps below steers both halves at once. */

void Menu_Glue_GetMouseState (float *x, float *y)
{
	if (x)
		*x = (float)ctest_menu_mouse_x;
	if (y)
		*y = (float)ctest_menu_mouse_y;
}

qboolean Menu_Glue_RayQuery (void)
{
	return vulkan_globals.ray_query;
}

qboolean Menu_Glue_SampleRateShading (void)
{
	return vulkan_globals.device_features.sampleRateShading;
}

float Menu_Glue_MaxSamplerAnisotropy (void)
{
	return vulkan_globals.device_properties.limits.maxSamplerAnisotropy;
}

const char *Menu_Glue_EngineNameAndVer (void)
{
	return ENGINE_NAME_AND_VER;
}

void ctest_menu_set_caps (qboolean ray_query, qboolean sample_rate_shading, float max_aniso)
{
	vulkan_globals.ray_query = ray_query;
	vulkan_globals.device_features.sampleRateShading = sample_rate_shading ? 1u : 0u;
	vulkan_globals.device_properties.limits.maxSamplerAnisotropy = max_aniso;
}

/* ---------------------------------------------------------------------------
 * host_cmd.c's four file-list accessors and net_main.c's three server-list
 * accessors. Both files are ported, and both halves of the menu differential
 * reach the same Rust body -- host_cmd_glue.c:711-746 and net_main_glue.c
 * define these plain names in the shipping build, and neither glue file is
 * compiled here, so menu.c is their first caller in this link. Shared by
 * construction, exactly as sbar_ref.c shares PR_ExecuteProgram: what is under
 * comparison is menu.c's marshalling, not the accessor.
 */

extern unsigned int quake_rs_hostcmd_extra_maps_get_type (const filelist_item_t *item);
extern const char  *quake_rs_hostcmd_extra_maps_get_message (const filelist_item_t *item);
extern qboolean		quake_rs_hostcmd_extra_maps_is_start (unsigned int type);
extern const char  *quake_rs_hostcmd_modlist_get_full_name (const filelist_item_t *item);

extern void		   rust_net_SlistSort (void);
extern const char *rust_net_SlistPrintServer (size_t idx);
extern const char *rust_net_SlistPrintServerName (size_t idx);

maptype_t ExtraMaps_GetType (const filelist_item_t *item)
{
	return (maptype_t)quake_rs_hostcmd_extra_maps_get_type (item);
}

const char *ExtraMaps_GetMessage (const filelist_item_t *item)
{
	return quake_rs_hostcmd_extra_maps_get_message (item);
}

qboolean ExtraMaps_IsStart (maptype_t type)
{
	return quake_rs_hostcmd_extra_maps_is_start ((unsigned int)type);
}

const char *Modlist_GetFullName (const filelist_item_t *item)
{
	return quake_rs_hostcmd_modlist_get_full_name (item);
}

void NET_SlistSort (void)
{
	rust_net_SlistSort ();
}

const char *NET_SlistPrintServer (size_t idx)
{
	return rust_net_SlistPrintServer (idx);
}

const char *NET_SlistPrintServerName (size_t idx)
{
	return rust_net_SlistPrintServerName (idx);
}

/* =========================================================================
 * FIXTURES
 *
 * side == 1 drives the C oracle through its c_ref_M_* name, side == 0 drives
 * quake-capi's port through quake_rs_menu_* directly. No plain M_* name is
 * defined or called here; Quake/menu_glue.c's plain-name layer is verified by
 * the engine link, not by this file.
 * ========================================================================= */

extern void		quake_rs_menu_get_crosshair_def (float crosshair_def_value, crosshair_t *out);
extern float	quake_rs_menu_get_scale (void);
extern void		quake_rs_menu_print (void *cbx, int cx, int cy, const char *str);
extern void		quake_rs_menu_draw_trans_pic (void *cbx, int x, int y, void *pic);
extern void		quake_rs_menu_draw_pic (void *cbx, int x, int y, void *pic);
extern void		quake_rs_menu_menu_changed (void);
extern int		quake_rs_menu_toggle_menu_f (void);
extern qboolean quake_rs_menu_handle_scroll_bar_keys (int key, int *cursor, int *first_drawn, int num_total, int max_on_screen);
extern void		quake_rs_menu_mouse_update_cursor (int *cursor, int left, int right, int top, int item_height, int index);
extern void		quake_rs_menu_menu_main_f (void);
extern void		quake_rs_menu_menu_options_f (void);
extern void		quake_rs_menu_menu_quit_f (void);
extern void		quake_rs_menu_check_mods (void);
extern void		quake_rs_menu_new_game (void);
extern int		quake_rs_menu_update_mouse (void);
extern int		quake_rs_menu_draw (void *cbx);
extern int		quake_rs_menu_keydown (int key);
extern void		quake_rs_menu_charinput (int key);
extern bool		quake_rs_menu_text_entry (void);
extern bool		quake_rs_menu_waiting_for_key_binding (void);
extern void		quake_rs_menu_menu_singleplayer_f (void);
extern void		quake_rs_menu_menu_load_f (void);
extern void		quake_rs_menu_menu_save_f (void);
extern void		quake_rs_menu_menu_maps_cmd_f (void);
extern void		quake_rs_menu_menu_multiplayer_f (void);
extern void		quake_rs_menu_menu_setup_f (void);
extern void		quake_rs_menu_menu_keys_f (void);
extern void		quake_rs_menu_menu_help_f (void);

/* ---- per-side glue-owned state ---------------------------------------- */

void ctest_menu_set_state (int side, int v)
{
	if (side)
		c_ref_m_state = (enum m_state_e)v;
	else
		m_state = v;
}

int ctest_menu_get_state (int side)
{
	return side ? (int)c_ref_m_state : m_state;
}

void ctest_menu_set_return_state (int side, int v)
{
	if (side)
		c_ref_m_return_state = (enum m_state_e)v;
	else
		m_return_state = v;
}

int ctest_menu_get_return_state (int side)
{
	return side ? (int)c_ref_m_return_state : m_return_state;
}

void ctest_menu_set_entersound (int side, qboolean v)
{
	if (side)
		c_ref_m_entersound = v;
	else
		m_entersound = v;
}

qboolean ctest_menu_get_entersound (int side)
{
	return side ? c_ref_m_entersound : m_entersound;
}

void ctest_menu_set_is_quitting (int side, qboolean v)
{
	if (side)
		c_ref_m_is_quitting = v;
	else
		m_is_quitting = v;
}

qboolean ctest_menu_get_is_quitting (int side)
{
	return side ? c_ref_m_is_quitting : m_is_quitting;
}

void ctest_menu_set_return_onerror (int side, qboolean v)
{
	if (side)
		c_ref_m_return_onerror = v;
	else
		m_return_onerror = v;
}

qboolean ctest_menu_get_return_onerror (int side)
{
	return side ? c_ref_m_return_onerror : m_return_onerror;
}

void ctest_menu_set_return_reason (int side, const char *s)
{
	char *dst = side ? c_ref_m_return_reason : m_return_reason;
	c_ref_q_strlcpy (dst, s ? s : "", 32);
}

const char *ctest_menu_get_return_reason (int side)
{
	return side ? c_ref_m_return_reason : m_return_reason;
}

/* ---- the four raise-capable entry points -------------------------------
 *
 * Return 0 when the call returned normally and 1 when it raised, by either
 * mechanism: the oracle half escapes through the harness Host_Error trap,
 * the port half returns a nonzero status out of a Menu_Glue_ guard above and
 * Quake/menu_glue.c re-raises it. Both leave the message in
 * ctest_host_error_message ().
 */

static int ctest_menu_side;
static int ctest_menu_key;
static int ctest_menu_status;

static void ctest_menu_invoke_toggle_menu (void *p)
{
	(void)p;
	if (ctest_menu_side)
		c_ref_M_ToggleMenu_f ();
	else
		ctest_menu_status = quake_rs_menu_toggle_menu_f ();
}

int ctest_menu_toggle_menu_f (int side)
{
	ctest_menu_side = side;
	ctest_menu_status = 0;
	if (ctest_try_host (ctest_menu_invoke_toggle_menu, NULL))
		return 1;
	return ctest_menu_status ? 1 : 0;
}

static void ctest_menu_invoke_update_mouse (void *p)
{
	(void)p;
	if (ctest_menu_side)
		c_ref_M_UpdateMouse ();
	else
		ctest_menu_status = quake_rs_menu_update_mouse ();
}

int ctest_menu_update_mouse (int side)
{
	ctest_menu_side = side;
	ctest_menu_status = 0;
	if (ctest_try_host (ctest_menu_invoke_update_mouse, NULL))
		return 1;
	return ctest_menu_status ? 1 : 0;
}

static void ctest_menu_invoke_draw (void *p)
{
	(void)p;
	if (ctest_menu_side)
		c_ref_M_Draw (NULL);
	else
		ctest_menu_status = quake_rs_menu_draw (NULL);
}

int ctest_menu_draw (int side)
{
	ctest_menu_side = side;
	ctest_menu_status = 0;
	if (ctest_try_host (ctest_menu_invoke_draw, NULL))
		return 1;
	return ctest_menu_status ? 1 : 0;
}

static void ctest_menu_invoke_keydown (void *p)
{
	(void)p;
	if (ctest_menu_side)
		c_ref_M_Keydown (ctest_menu_key);
	else
		ctest_menu_status = quake_rs_menu_keydown (ctest_menu_key);
}

int ctest_menu_keydown (int side, int key)
{
	ctest_menu_side = side;
	ctest_menu_key = key;
	ctest_menu_status = 0;
	if (ctest_try_host (ctest_menu_invoke_keydown, NULL))
		return 1;
	return ctest_menu_status ? 1 : 0;
}

/* ---- the non-raising public surface ------------------------------------ */

void ctest_menu_charinput (int side, int key)
{
	if (side)
		c_ref_M_Charinput (key);
	else
		quake_rs_menu_charinput (key);
}

qboolean ctest_menu_text_entry (int side)
{
	return side ? c_ref_M_TextEntry () : quake_rs_menu_text_entry ();
}

qboolean ctest_menu_waiting_for_key_binding (int side)
{
	return side ? c_ref_M_WaitingForKeyBinding () : quake_rs_menu_waiting_for_key_binding ();
}

float ctest_menu_get_scale (int side)
{
	return side ? c_ref_M_GetScale () : quake_rs_menu_get_scale ();
}

void ctest_menu_get_crosshair_def (int side, float crosshair_def_value, crosshair_t *out)
{
	if (side)
		*out = c_ref_M_GetCrosshairDef (crosshair_def_value);
	else
		quake_rs_menu_get_crosshair_def (crosshair_def_value, out);
}

void ctest_menu_print (int side, int cx, int cy, const char *str)
{
	if (side)
		c_ref_M_Print (NULL, cx, cy, str);
	else
		quake_rs_menu_print (NULL, cx, cy, str);
}

void ctest_menu_draw_pic (int side, int x, int y, const char *pic)
{
	qpic_t *p = ctest_draw_pic (pic);

	if (side)
		c_ref_M_DrawPic (NULL, x, y, p);
	else
		quake_rs_menu_draw_pic (NULL, x, y, p);
}

void ctest_menu_draw_trans_pic (int side, int x, int y, const char *pic)
{
	qpic_t *p = ctest_draw_pic (pic);

	if (side)
		c_ref_M_DrawTransPic (NULL, x, y, p);
	else
		quake_rs_menu_draw_trans_pic (NULL, x, y, p);
}

void ctest_menu_menu_changed (int side)
{
	if (side)
		c_ref_M_MenuChanged ();
	else
		quake_rs_menu_menu_changed ();
}

qboolean ctest_menu_handle_scroll_bar_keys (int side, int key, int *cursor, int *first_drawn, int num_total, int max_on_screen)
{
	if (side)
		return c_ref_M_HandleScrollBarKeys (key, cursor, first_drawn, num_total, max_on_screen);
	return quake_rs_menu_handle_scroll_bar_keys (key, cursor, first_drawn, num_total, max_on_screen);
}

void ctest_menu_mouse_update_cursor (int side, int *cursor, int left, int right, int top, int item_height, int index)
{
	if (side)
		c_ref_M_Mouse_UpdateCursor (cursor, left, right, top, item_height, index);
	else
		quake_rs_menu_mouse_update_cursor (cursor, left, right, top, item_height, index);
}

void ctest_menu_check_mods (int side)
{
	if (side)
		c_ref_M_CheckMods ();
	else
		quake_rs_menu_check_mods ();
}

void ctest_menu_new_game (int side)
{
	if (side)
		c_ref_M_NewGame ();
	else
		quake_rs_menu_new_game ();
}

/* ---- the menu_* command entry points ----------------------------------
 *
 * Every one of these is a Cmd_AddCommand target in M_Init, so each can raise
 * through the same indirect surface the four above reach; each is dispatched
 * through ctest_try_host for the same reason.
 */

typedef void (*ctest_menu_cmd_t) (void);

static ctest_menu_cmd_t ctest_menu_cmd;

static void ctest_menu_invoke_cmd (void *p)
{
	(void)p;
	ctest_menu_cmd ();
}

static int ctest_menu_run_cmd (ctest_menu_cmd_t fn)
{
	ctest_menu_cmd = fn;
	return ctest_try_host (ctest_menu_invoke_cmd, NULL) ? 1 : 0;
}

int ctest_menu_menu_main_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Main_f : quake_rs_menu_menu_main_f);
}

int ctest_menu_menu_options_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Options_f : quake_rs_menu_menu_options_f);
}

int ctest_menu_menu_quit_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Quit_f : quake_rs_menu_menu_quit_f);
}

int ctest_menu_menu_singleplayer_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_SinglePlayer_f : quake_rs_menu_menu_singleplayer_f);
}

int ctest_menu_menu_load_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Load_f : quake_rs_menu_menu_load_f);
}

int ctest_menu_menu_save_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Save_f : quake_rs_menu_menu_save_f);
}

int ctest_menu_menu_maps_cmd_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Maps_Cmd_f : quake_rs_menu_menu_maps_cmd_f);
}

int ctest_menu_menu_multiplayer_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_MultiPlayer_f : quake_rs_menu_menu_multiplayer_f);
}

int ctest_menu_menu_setup_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Setup_f : quake_rs_menu_menu_setup_f);
}

int ctest_menu_menu_keys_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Keys_f : quake_rs_menu_menu_keys_f);
}

int ctest_menu_menu_help_f (int side)
{
	return ctest_menu_run_cmd (side ? c_ref_M_Menu_Help_f : quake_rs_menu_menu_help_f);
}

/* ---- the command argument vector --------------------------------------
 *
 * M_Menu_Maps_Cmd_f (menu.c:3020) reads Cmd_Argc / Cmd_Argv, and cmd.c's
 * oracle and quake-capi's port keep separate argument vectors, so each side
 * has to be tokenized into its own.
 */

void ctest_menu_tokenize (int side, const char *text)
{
	if (side)
		c_ref_Cmd_TokenizeString (text);
	else
		Cmd_TokenizeString (text);
}

/* =========================================================================
 * WORLD-STATE FIXTURES
 *
 * Shared objects get one setter; the ones c_ref_prelude.h splits get a
 * per-side setter. The split is not a judgement call -- it is whatever the
 * prelude did, recorded here so the test never has to guess:
 *
 *   shared  vid, glwidth, glheight, scr_con_current, realtime,
 *           hostCacheCount (:116, but see ctest_menu_set_net),
 *           host_rawframetime, key_dest, keydown (stubs/keys_ref.c:223),
 *           slistInProgress, multiuser, vulkan_globals
 *   split   cl / cls (:1767, :1806), sv / svs (:1145-1146),
 *           net_hostport (:172),
 *           ipv4Available / ipv6Available (:175-176), cmd_text (:330)
 * ========================================================================= */

/* cmd.c:70 defines cmd_text as a global with no header declaration, so the
 * renamed oracle copy has to be declared here by hand. */
extern sizebuf_t c_ref_cmd_text;

#undef cmd_text
#undef sv
#undef svs
#undef hostCacheCount
#undef net_hostport
#undef ipv4Available
#undef ipv6Available
#undef com_gamedir

extern sizebuf_t		cmd_text;
extern server_t			sv;
extern server_static_t	svs;
extern size_t			hostCacheCount;
extern int				net_hostport;
extern qboolean			ipv4Available;
extern qboolean			ipv6Available;
extern char				com_gamedir[MAX_OSPATH];

void ctest_menu_set_screen (int vid_w, int vid_h, int glw, int glh, float con_current)
{
	vid.width = vid_w;
	vid.height = vid_h;
	glwidth = glw;
	glheight = glh;
	scr_con_current = con_current;
}

void ctest_menu_set_time (double now, double rawframetime)
{
	realtime = now;
	host_rawframetime = rawframetime;
}

void ctest_menu_set_key_dest (int dest)
{
	key_dest = (keydest_t)dest;
}

int ctest_menu_get_key_dest (void)
{
	return (int)key_dest;
}

void ctest_menu_set_key_down (int key, qboolean down)
{
	if (key >= 0 && key < MAX_KEYS)
		keydown[key] = down;
}

void ctest_menu_clear_key_down (void)
{
	memset (keydown, 0, MAX_KEYS * sizeof (keydown[0]));
}

/* Key bindings. c_ref_prelude.h does NOT rename `keybindings` or
 * Key_SetBinding -- stubs/keys_ref.c:73 and :90 do that locally, inside keys.c's
 * own TU -- so the menu.c composed above and quake-capi's port both read the
 * SAME plain table (defined at stubs/keys_ref.c:220, which is the object
 * Quake/keys_glue.c owns in the engine). One setter therefore seeds both
 * halves, and M_UnbindCommand's writes from one side are visible to the other:
 * the test re-seeds before each side runs and snapshots immediately after, so
 * what is compared is still each half's own effect.
 */
void ctest_menu_bind (int keynum, const char *command)
{
	if (keynum >= 0 && keynum < MAX_KEYS)
		Key_SetBinding (keynum, command);
}

void ctest_menu_clear_binds (void)
{
	int i;

	for (i = 0; i < MAX_KEYS; i++)
		Key_SetBinding (i, NULL);
}

/* slist_scope / slist_silent (stubs/stubs.c:3184, net_main.c) are NOT
 * renamed by c_ref_prelude.h, so menu.c:4417-4418 writes the SAME object on
 * both halves. That is still comparable -- the test snapshots immediately
 * after each side runs, before the other side overwrites it -- so the scope
 * the search menu selects is part of the observation rather than a shared
 * object nobody looks at. */
int ctest_menu_get_slist_scope (void)
{
	return (int)slist_scope;
}

qboolean ctest_menu_get_slist_silent (void)
{
	return slist_silent;
}

void ctest_menu_set_misc (qboolean slist_in_progress, qboolean multiuser_value)
{
	slistInProgress = slist_in_progress;
	multiuser = multiuser_value;
}

/* svs.clients has to be real storage, not NULL: menu.c writes hostname, and
 * cvar.c:507-517 walks svs.clients[0 .. svs.maxclients) for every
 * CVAR_SERVERINFO write. With a null base that walk faults before either
 * half can be compared. stubs/pf_msg_ref.c:139 and stubs/host_ref.c:520
 * bind private arrays for the same reason; these two are this file's. */
#define CTEST_MENU_MAX_CLIENTS 8

static client_t ctest_menu_c_clients[CTEST_MENU_MAX_CLIENTS];
static client_t ctest_menu_r_clients[CTEST_MENU_MAX_CLIENTS];

void ctest_menu_set_server (int side, qboolean active, int maxclients, int maxclientslimit)
{
	if (maxclients < 0)
		maxclients = 0;
	if (maxclients > CTEST_MENU_MAX_CLIENTS)
		maxclients = CTEST_MENU_MAX_CLIENTS;
	if (side)
	{
		memset (ctest_menu_c_clients, 0, sizeof (ctest_menu_c_clients));
		c_ref_sv.active = active;
		c_ref_svs.clients = ctest_menu_c_clients;
		c_ref_svs.maxclients = maxclients;
		c_ref_svs.maxclientslimit = maxclientslimit;
	}
	else
	{
		memset (ctest_menu_r_clients, 0, sizeof (ctest_menu_r_clients));
		sv.active = active;
		svs.clients = ctest_menu_r_clients;
		svs.maxclients = maxclients;
		svs.maxclientslimit = maxclientslimit;
	}
}

void ctest_menu_set_client (int side, int state, int demonum, qboolean demoplayback, int signon, int intermission, const char *mapname)
{
	client_static_t *s = side ? &c_ref_cls : &cls;
	client_state_t	*c = side ? &c_ref_cl : &cl;

	s->state = (cactive_t)state;
	s->demonum = demonum;
	s->demoplayback = demoplayback;
	s->signon = signon;
	c->intermission = intermission;
	c_ref_q_strlcpy (c->mapname, mapname ? mapname : "", sizeof (c->mapname));
}

int ctest_menu_get_demonum (int side)
{
	return side ? c_ref_cls.demonum : cls.demonum;
}

/* hostCacheCount is the one "split" object that has to be written on BOTH
 * sides every time. menu.c's own reads of it are split -- the oracle sees
 * stubs.c:3041 through c_ref_prelude.h:116, the port sees quake-capi's --
 * but the three readers menu.c calls to turn an index into a string,
 * NET_SlistSort / NET_SlistPrintServer / NET_SlistPrintServerName (:1284
 * below), are NOT renamed by the prelude: there is one definition, it is
 * the port's, and it bounds-checks against the PLAIN count
 * (net_main.c:508, :526). Seeding only c_ref_hostCacheCount therefore gives
 * the oracle a loop that runs the right number of times over strings that
 * are all empty, which is a harness artefact and not a divergence. The
 * other three stay per side: nothing shared reads them here. */
/* M_ScanSaves (menu.c:846) opens "%s/s%i.sav" under com_gamedir with
 * Sys_fopen -- a plain path, not a search-path lookup -- so pointing the two
 * com_gamedirs at a directory that really holds an s*.sav is all it takes to
 * give the slot table a loadable row. c_ref_prelude.h:406 renames com_gamedir,
 * so the two halves have separate copies and this is a per-side setter
 * (stubs/console_ref.c:1672 does the same for Con_Dump_f). */
void ctest_menu_set_gamedir (int side, const char *dir)
{
	c_ref_q_strlcpy (side ? c_ref_com_gamedir : com_gamedir, dir ? dir : "", MAX_OSPATH);
}

void ctest_menu_set_net (int side, int cachecount, qboolean ipv4, qboolean ipv6, int hostport)
{
	c_ref_hostCacheCount = (size_t)cachecount;
	hostCacheCount = (size_t)cachecount;
	if (side)
	{
		c_ref_ipv4Available = ipv4;
		c_ref_ipv6Available = ipv6;
		c_ref_net_hostport = hostport;
	}
	else
	{
		ipv4Available = ipv4;
		ipv6Available = ipv6;
		net_hostport = hostport;
	}
}

/* ---------------------------------------------------------------------------
 * The command buffer. Both halves have one (stubs/stubs.c:1626 for the port,
 * cmd.c's own for the oracle), and half of what a menu key handler does is
 * append to it, so the pending text is a primary observation. Reading it
 * rather than executing it keeps the observation free of whatever the two
 * command registries would or would not have found.
 */

/* c_ref_prelude.h:335 renames Cbuf_Init, so without this the plain call
 * below would be a second call to the oracle and the port's cmd_text would
 * keep maxsize 0 -- every Cbuf_AddText a menu performs would then be an
 * "overflow" warning instead of a write. */
#undef Cbuf_Init
extern void Cbuf_Init (void);

void ctest_menu_cbuf_init (void)
{
	static qboolean done;

	if (done)
		return;
	done = true;
	c_ref_Cbuf_Init ();
	Cbuf_Init ();
}

void ctest_menu_cbuf_clear (int side)
{
	if (side)
		c_ref_cmd_text.cursize = 0;
	else
		cmd_text.cursize = 0;
}

const char *ctest_menu_cbuf_text (int side)
{
	static char buf[8192];
	sizebuf_t  *sb = side ? &c_ref_cmd_text : &cmd_text;
	int			n = sb->cursize;

	if (n < 0)
		n = 0;
	if (n > (int)sizeof (buf) - 1)
		n = (int)sizeof (buf) - 1;
	if (n > 0 && sb->data)
		memcpy (buf, sb->data, (size_t)n);
	buf[n] = '\0';
	return buf;
}

/* ---------------------------------------------------------------------------
 * Cvar seeding. The nineteen this file registers go through Cvar_Set, so the
 * registry, the callback path and the string allocation all behave exactly as
 * they do in the engine. The other thirty-four are registered in NEITHER
 * registry in this link (verified: ctest_menu_cvar_registered reports 0 on
 * both sides for all thirty-four), so a by-name write from menu.c fails
 * identically on both sides -- which is itself part of the differential -- and
 * seeding them means writing the fields directly. Cvar_SetQuick is not usable
 * there: an unregistered cvar_t still points ->string at its initializer, and
 * freeing a string literal is not a thing this harness is going to do.
 */

static char ctest_menu_cvar_seed_buf[CTEST_MENU_NUM_CVARS][64];

void ctest_menu_cvar_seed (int side, int idx, const char *value)
{
	cvar_t *var;

	if (idx < 0 || idx >= CTEST_MENU_NUM_CVARS || !value)
		return;
	var = side ? ctest_menu_cvar_table[idx].c : ctest_menu_cvar_table[idx].r;
	if (ctest_menu_cvar_table[idx].def)
	{
		if (side)
			c_ref_Cvar_Set (var->name, value);
		else
			Cvar_Set (var->name, value);
		return;
	}
	c_ref_q_strlcpy (ctest_menu_cvar_seed_buf[idx], value, sizeof (ctest_menu_cvar_seed_buf[idx]));
	var->string = ctest_menu_cvar_seed_buf[idx];
	var->value = (float)atof (value);
}

/* ---------------------------------------------------------------------------
 * The two file lists menu.c walks: modlist (:2593, :2607, :2644) and
 * extralevels_sorted (:2978-2982). Both heads are plain C objects in this link
 * (stubs/host_cmd_glue_ref.c:118-122) and both halves of the differential read
 * the same ones, so one setter serves both sides.
 *
 * FileList_AddEx over-allocates each node with a payload the accessors reach
 * by item+1: levelinfo_t {type, message} for extralevels (host_cmd.c:262-266,
 * mirrored at quake-capi/src/host_cmd.rs:777) and modinfo_t {char[64]} for
 * modlist (host_cmd.c:569-572, mirrored at :1209). The node structs below
 * reproduce that layout so quake_rs_hostcmd_extra_maps_get_type and
 * _modlist_get_full_name read a real payload instead of whatever follows a
 * bare filelist_item_t.
 */

#define CTEST_MENU_MAX_LEVELS 16
#define CTEST_MENU_MAX_MODS	  8

typedef struct
{
	filelist_item_t item;
	unsigned int	type;
	const char	   *message;
} ctest_menu_level_node_t;

typedef struct
{
	filelist_item_t item;
	char			full_name[64];
} ctest_menu_mod_node_t;

static ctest_menu_level_node_t ctest_menu_levels[CTEST_MENU_MAX_LEVELS];
static char					   ctest_menu_level_msgs[CTEST_MENU_MAX_LEVELS][64];
static filelist_item_t		  *ctest_menu_levels_sorted[CTEST_MENU_MAX_LEVELS + 1];
static int					   ctest_menu_level_count;

static ctest_menu_mod_node_t ctest_menu_mods[CTEST_MENU_MAX_MODS];
static int					 ctest_menu_mod_count;

void ctest_menu_clear_levels (void)
{
	memset (ctest_menu_levels, 0, sizeof (ctest_menu_levels));
	memset (ctest_menu_level_msgs, 0, sizeof (ctest_menu_level_msgs));
	memset (ctest_menu_levels_sorted, 0, sizeof (ctest_menu_levels_sorted));
	ctest_menu_level_count = 0;
	extralevels = NULL;
	extralevels_sorted = NULL;
}

void ctest_menu_add_level (const char *name, unsigned int type, const char *message)
{
	int i = ctest_menu_level_count;

	if (i >= CTEST_MENU_MAX_LEVELS)
		return;
	c_ref_q_strlcpy (ctest_menu_levels[i].item.name, name, sizeof (ctest_menu_levels[i].item.name));
	ctest_menu_levels[i].item.next = NULL;
	ctest_menu_levels[i].type = type;
	if (message)
	{
		c_ref_q_strlcpy (ctest_menu_level_msgs[i], message, sizeof (ctest_menu_level_msgs[i]));
		ctest_menu_levels[i].message = ctest_menu_level_msgs[i];
	}
	else
		ctest_menu_levels[i].message = NULL;
	if (i)
		ctest_menu_levels[i - 1].item.next = &ctest_menu_levels[i].item;
	ctest_menu_levels_sorted[i] = &ctest_menu_levels[i].item;
	ctest_menu_levels_sorted[i + 1] = NULL;
	ctest_menu_level_count = i + 1;
	extralevels = &ctest_menu_levels[0].item;
	extralevels_sorted = ctest_menu_levels_sorted;
}

void ctest_menu_clear_mods (void)
{
	memset (ctest_menu_mods, 0, sizeof (ctest_menu_mods));
	ctest_menu_mod_count = 0;
	modlist = NULL;
}

void ctest_menu_add_mod (const char *name, const char *full_name)
{
	int i = ctest_menu_mod_count;

	if (i >= CTEST_MENU_MAX_MODS)
		return;
	c_ref_q_strlcpy (ctest_menu_mods[i].item.name, name, sizeof (ctest_menu_mods[i].item.name));
	ctest_menu_mods[i].item.next = NULL;
	c_ref_q_strlcpy (ctest_menu_mods[i].full_name, full_name ? full_name : "", sizeof (ctest_menu_mods[i].full_name));
	if (i)
		ctest_menu_mods[i - 1].item.next = &ctest_menu_mods[i].item;
	ctest_menu_mod_count = i + 1;
	modlist = &ctest_menu_mods[0].item;
}
