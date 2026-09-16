# Day Tunes UI strings (https://daybrite.dev/docs/localization). Add a locale by dropping a
# sibling folder (e.g. locales/fr/app.ftl) and translating; the generated
# res::locales::install() in src/lib.rs picks up every locale directory by itself.
#
# The appearance and language rows on the Settings page label themselves from Day's own catalog,
# so there are no keys for them here.

app_title = Day Tunes

# The sections. Browse, Search, Library, and Now Playing are tabs on a phone and sidebar rows on
# a desktop; Settings is a row only where there is no menu bar.
nav_browse = Browse
nav_search = Search
nav_library = Library
nav_playing = Now Playing
nav_settings = Settings

# Menus and commands. One string per command, shared by the menu bar, the toolbar, and the
# now-playing page, so a command reads the same wherever the user finds it.
menu_file = File
menu_edit = Edit
menu_playback = Playback
cmd_play = Play
cmd_pause = Pause
cmd_stop = Stop
cmd_favorite = Add to Favorites
cmd_unfavorite = Remove from Favorites

# The Browse pane: how the catalog is cut, and the rows inside each cut.
browse_top = Top
browse_countries = Countries
browse_genres = Genres
browse_languages = Languages
browse_top_title = Top stations
browse_count = { $count } stations

# The Search pane.
search_hint = Search stations…
search_empty = Type to search { $count } stations by name, genre, or country.
search_none = No stations match “{ $query }”.

# The Library pane.
library_favorites = Favorites
library_recents = Recents
library_no_favorites = Stations you favorite appear here.
library_no_recents = Stations you play appear here.

# The station page.
station_none = Select a station
station_play = Play
station_play_variant = Play { $label }
station_homepage = Visit homepage
station_online = Online
station_offline = Offline
station_status = Status
station_votes = Votes
station_stream = Stream
station_languages = Languages
station_nature = Nature
station_about = About
station_bitrate = { $kbps } kbps
station_hls = HLS

# The now-playing page and the transport.
playing_nothing = Nothing playing
playing_pick = Pick a station in Browse or Search to start listening.
playing_state_idle = Stopped
playing_state_loading = Connecting…
playing_state_playing = Live
playing_state_paused = Paused
playing_state_ended = Stream ended
playing_state_error = Could not play: { $reason }
playing_open = Open station

# Genre names, one per canonical tag in the catalog (data/README.md). Looked up by slug, so a
# tag the catalog adds later shows as its slug until it gets a line here.
tag_2000s = 2000s
tag_2010s = 2010s
tag_50s = 50s
tag_60s = 60s
tag_70s = 70s
tag_80s = 80s
tag_90s = 90s
tag_adult_contemporary = Adult Contemporary
tag_alternative = Alternative
tag_ambient = Ambient
tag_anime = Anime
tag_arabic_music = Arabic
tag_ballad = Ballads
tag_blues = Blues
tag_bollywood = Bollywood
tag_business = Business
tag_catholic = Catholic
tag_chillout = Chillout
tag_christian_music = Christian
tag_classic_hits = Classic Hits
tag_classic_rock = Classic Rock
tag_classical = Classical
tag_comedy = Comedy
tag_community_radio = Community Radio
tag_country = Country
tag_culture = Culture
tag_cumbia = Cumbia
tag_dance = Dance
tag_disco = Disco
tag_downtempo = Downtempo
tag_drum_and_bass = Drum & Bass
tag_dubstep = Dubstep
tag_edm = EDM
tag_education = Education
tag_electronic = Electronic
tag_experimental = Experimental
tag_folk = Folk
tag_funk = Funk
tag_gospel = Gospel
tag_hard_rock = Hard Rock
tag_hardcore = Hardcore
tag_hip_hop = Hip-Hop
tag_hits = Hits
tag_house = House
tag_indie = Indie
tag_instrumental = Instrumental
tag_islamic = Islamic
tag_j_pop = J-Pop
tag_jazz = Jazz
tag_k_pop = K-Pop
tag_kids = Kids
tag_latin = Latin
tag_lifestyle = Lifestyle
tag_local_news = Local News
tag_lofi = Lo-Fi
tag_lounge = Lounge
tag_merengue = Merengue
tag_metal = Metal
tag_new_wave = New Wave
tag_news = News
tag_news_talk = News & Talk
tag_oldies = Oldies
tag_opera = Opera
tag_party = Party
tag_podcast = Podcast
tag_politics = Politics
tag_pop = Pop
tag_pop_rock = Pop Rock
tag_prog_rock = Prog Rock
tag_public_radio = Public Radio
tag_punk = Punk
tag_r_and_b = R&B
tag_rap = Rap
tag_reggae = Reggae
tag_reggaeton = Reggaeton
tag_regional_mexican = Regional Mexican
tag_religious = Religious
tag_retro = Retro
tag_rock = Rock
tag_romantic = Romantic
tag_salsa = Salsa
tag_ska = Ska
tag_sleep = Sleep
tag_smooth_jazz = Smooth Jazz
tag_soft_rock = Soft Rock
tag_soul = Soul
tag_soundtrack = Soundtrack
tag_sports = Sports
tag_sports_talk = Sports Talk
tag_synthpop = Synthpop
tag_talk = Talk
tag_techno = Techno
tag_top_40 = Top 40
tag_trance = Trance
tag_tropical = Tropical
tag_world = World

# The station catalog: where the stations come from, and how the copy on this device is kept
# current (src/sync.rs). The first launch shows these in place of the lists.
catalog_section = Station catalog
catalog_fetching = Getting the station catalog
catalog_idle = Waiting to check for the catalog.
catalog_checking = Checking for a catalog update…
catalog_downloading = Downloading { $progress }…
catalog_verifying = Verifying the download…
catalog_up_to_date = The catalog is up to date.
catalog_failed = The catalog could not be fetched: { $reason }
catalog_retry = Try Again
catalog_check = Check for Updates
catalog_installed = Installed
catalog_installed_line = { $count } stations, published { $date }
catalog_none = No catalog yet
catalog_status = Status
catalog_source = Source
catalog_source_hint = Publisher URL (blank for the default)

# The listener's own words on a station page: a note, and tags the Library can filter by.
station_notes = Notes
station_notes_hint = Your notes about this station
station_tags_title = Your tags
station_tag_hint = Add a tag…
station_tag_add = Add
library_all_tags = All tags
library_missing = Not in this catalog

# The Now Playing transport.
playing_previous = Previous station
playing_next = Next station
playing_track_none = Waiting for the stream to say what is playing
