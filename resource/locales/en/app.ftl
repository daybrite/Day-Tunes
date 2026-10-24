# Day Tunes UI strings (https://daybrite.dev/docs/how-to/localization). Add a locale by dropping a
# sibling folder (e.g. locales/fr/app.ftl) and translating; the generated
# res::locales::install() in src/lib.rs picks up every locale directory by itself.
#
# The appearance and language rows on the Settings page label themselves from Day's own catalog,
# so there are no keys for them here.

app_title = Day Tunes
about_tagline = Your music, the world's radio, and your podcasts, in one player.

# The sections: sidebar rows on a desktop, tabs and stack pages on a phone.
nav_library = Library
nav_songs = Songs
nav_albums = Albums
nav_artists = Artists
nav_genres = Genres
nav_recently_added = Recently Added
nav_history = Recently Played
nav_queue = Up Next
nav_playlists = Playlists
nav_radio = Radio
nav_stations = Stations
nav_browse = Browse
nav_podcasts = Podcasts
nav_episodes = Episodes
nav_shows = Shows
nav_discover = Discover
nav_downloads = Downloads
nav_playing = Now Playing
# The web build, which has no library, groups its queue and history under this header.
nav_listening = Listening
nav_settings = Settings

# Menus and commands. One string per command, shared by the menu bar, the toolbars, the
# context menus and the pages, so a command reads the same wherever the listener finds it.
menu_file = File
menu_edit = Edit
menu_controls = Controls
menu_view = View
cmd_play = Play
cmd_pause = Pause
cmd_stop = Stop
cmd_next = Next
cmd_previous = Previous
cmd_shuffle = Shuffle
cmd_shuffle_all = Shuffle All
cmd_repeat = Repeat
cmd_volume_up = Volume Up
cmd_volume_down = Volume Down
cmd_find = Find
cmd_add = Add
cmd_add_folder = Add Folder…
cmd_import_files = Import Files…
cmd_rescan = Rescan Folders
cmd_new_playlist = New Playlist…
cmd_rename_playlist = Rename Playlist…
cmd_delete_playlist = Delete Playlist
cmd_add_to_playlist = Add to Playlist
cmd_remove_from_playlist = Remove from Playlist
cmd_play_next = Play Next
cmd_add_to_queue = Add to Up Next
cmd_remove_from_queue = Remove from Up Next
cmd_clear_queue = Clear
cmd_clear_history = Clear History
cmd_go_to_album = Go to Album
cmd_go_to_artist = Go to Artist
cmd_go_to_show = Go to Show
cmd_remove_from_library = Remove from Library…
cmd_remove = Remove
cmd_delete = Delete
cmd_more = More
cmd_sort = Sort
cmd_retry = Try Again
cmd_favorite = Add to Stations
cmd_unfavorite = Remove from Stations
cmd_vote = Vote
cmd_add_podcast = Add Podcast by URL…
cmd_refresh = Refresh
cmd_refresh_podcasts = Refresh Podcasts
cmd_subscribe = Subscribe
cmd_subscribed = Subscribed
cmd_unsubscribe = Unsubscribe
cmd_download = Download
cmd_remove_download = Remove Download
cmd_mark_played = Mark as Played
cmd_mark_unplayed = Mark as Unplayed
cmd_mark_all_played = Mark All as Played
cmd_play_latest = Play Latest
cmd_sign_in = Sign In
cmd_sign_out = Sign Out

# The Songs list.
search_songs = Search songs, artists, albums
search_none = Nothing matches “{ $query }”.
sort_title = Title
sort_artist = Artist
sort_album = Album
sort_added = Recently Added
sort_played = Recently Played
library_empty = Your library is empty. Add a folder of music, or import files.
library_empty_phone = Import songs from your files to start a library.
genres_empty = Songs with a genre tag show up here.
unknown_artist = Unknown Artist
unknown_album = Unknown Album
track_missing = Missing track
track_count = { $count ->
    [one] 1 song
   *[other] { $count } songs
}
album_count = { $count ->
    [one] 1 album
   *[other] { $count } albums
}
disc_no = Disc { $n }
bitrate = { $n } kbps
scan_progress = Scanning { $folder }… { $seen } files
import_progress = Importing… { $count } added
recently_added_heading = Recently Added

# The detail pane's empty states.
detail_pick_song = Select a song
detail_pick_album = Select an album
detail_pick_artist = Select an artist
detail_pick_genre = Select a genre
detail_pick_station = Select a station
detail_pick_episode = Select an episode
detail_pick_show = Select a show

# Facts on a page.
section_details = Details
fact_album = Album
fact_artist = Artist
fact_album_artist = Album Artist
fact_genre = Genre
fact_year = Year
fact_track_no = Track
fact_duration = Duration
fact_format = Format
fact_plays = Plays
fact_added = Added
fact_file = File
fact_stream = Stream
fact_stream_url = Stream URL
fact_language = Language
fact_votes = Votes
vote_thanks = Thanks for voting!

# Playlists.
playlist_untitled = Untitled Playlist
playlist_empty = Add songs to this playlist from their menu.
playlists_empty = Playlists you make appear here.
new_playlist_title = New Playlist
new_playlist_hint = Name
rename_playlist_title = Rename Playlist
delete_playlist_title = Delete this playlist?
delete_playlist_message = The songs stay in your library.
remove_track_title = Remove this song from the library?
remove_track_message = A file you added to a folder stays where it is. An imported copy is deleted.
remove_folder_title = Remove this folder?
remove_folder_message = Its songs leave the library. The files stay on disk.
add_folder_title = Choose a folder of music
add_folder_caption = Scan a folder and keep it in the library.
import_files_title = Choose audio files
import_files_caption = Copy files into the app's own library.
audio_files = Audio files

# History and the queue.
history_empty = What you play appears here.
queue_empty = Nothing is queued. Play something, or add songs to Up Next.
queue_up_next = Up Next
queue_count = { $count ->
    [one] 1 item
   *[other] { $count } items
}

# Radio.
radio_top = Top
radio_trending = Trending
radio_countries = Countries
radio_tags = Tags
radio_languages = Languages
radio_search_title = Search: { $query }
search_stations = Search stations
stations_empty = Stations you keep, and the ones you play, appear here.
stations_favorites = My Stations
stations_recent = Recently Played
station_count = { $count ->
    [one] 1 station
   *[other] { $count } stations
}
station_homepage = Visit Website
browse_none = No stations here.
live = Live
loading = Loading…
fetch_failed = Could not load: { $error }

# What the player says.
state_idle = Stopped
state_loading = Connecting…
state_playing = Playing
state_paused = Paused
state_ended = Ended
state_error = Could not play: { $reason }
playing_nothing = Nothing playing
playing_pick = Pick a song, a station, or an episode.

# Podcasts.
search_podcasts = Search podcasts
discover_top = Top Podcasts
discover_none = No shows found.
episodes_empty = Episodes of the shows you follow appear here.
shows_empty = Follow a show from Discover, or add one by its feed URL.
episode_count = { $count ->
    [one] 1 episode
   *[other] { $count } episodes
}
episode_played = Played
episode_resume_at = Resume at { $time }
downloading = Downloading…
refreshing = Refreshing…
preview_latest = Latest Episodes
show_website = Website
add_podcast_title = Add Podcast
add_podcast_message = Paste the feed URL of a podcast.
add_podcast_failed = The podcast could not be added
unsubscribe_title = Unsubscribe from this show?
unsubscribe_message = Its episodes and downloads are removed.

# Settings.
settings_library = Library
settings_tracks = Songs
settings_radio = Radio
settings_radio_index = Index
settings_radio_server = Server
settings_radio_server_caption = A Radio Browser API host of your choice. Leave empty to pick one from the public list.
settings_radio_note = Station data comes from the community-run Radio Browser index.
settings_podcasts = Podcasts
settings_directory = Directory
settings_podcast_country = Storefront
settings_podcast_country_caption = The two-letter country whose chart Discover shows.
settings_refresh_at_launch = Refresh at launch
settings_last_error = Last problem
settings_scrobbling = Scrobbling
settings_service = Service
service_off = Off
service_lastfm = Last.fm
service_librefm = Libre.fm
service_listenbrainz = ListenBrainz
service_custom = Other GNU FM server
settings_custom_base = Server URL
settings_api_key = API key
settings_api_key_caption = Last.fm needs the key of an API account you created at last.fm/api. Libre.fm takes any key.
settings_api_secret = Shared secret
settings_username = Username
settings_password = Password
settings_token = User token
settings_account = Account
account_none = Not signed in
account_signing_in = Signing in…
account_signed_in = Signed in as { $user }
account_failed = Sign-in failed: { $error }
settings_scrobble_music = Scrobble songs
settings_scrobble_radio = Scrobble radio songs
settings_scrobble_podcasts = Scrobble episodes
settings_pending = Waiting to send
scrobble_off = Off
none = None

# Permission reasons (https://daybrite.dev/docs/reference/permissions).
permission_biometrics = Unlock the scrobbling account kept in the keychain.

# The background-playback notification (Android).
background_description = Playing in the background
background_channel = Playback
