# Independent microphone translation and Windows text insertion

The desktop currently shares recognition/translation languages between system
audio and the microphone. Add an explicit independent microphone-language
opt-in; preserve legacy behavior while it is off. Keep portable language
resolution in mimi-runtime, with desktop-only preference and capture wiring.

The destination defaults to the selected incoming/system speech language.
Automatic incoming recognition cannot specify a translation destination: require
an explicit incoming language or a manual microphone destination rather than
guessing, sending speech in the wrong language, or silently changing provider.
Validate each lane against its own route/credentials before opening capture.
Language edits require stopping the session. Both lanes retain independent
provider connections, final pairs and capture identity.

Windows insertion is an explicit per-utterance action in voice settings. The user
chooses “Insert microphone translation”, then focuses a target within three
seconds. Take text only from the native, confirmed microphone display pair;
never accept arbitrary IPC text or previews. Cancel on navigation, session
change, clear, or replacement of the chosen pair. Reject repeated successful
insertions of the same pair. UI Automation must confirm a focused, writable,
non-password external edit control immediately before Unicode SendInput.
Do not read target text, use the clipboard, activate another application,
inject Enter/Tab, submit forms, elevate, or retry partial input. Unsupported
editors fail visibly. This first action is manual, not continuous dictation.

Acceptance: English→Persian system subtitles and Persian→English microphone
subtitles coexist without changing each other; explicit targets work when
incoming recognition is automatic; unknown automatic incoming destinations
fail before capture. Preferences survive restart. Windows insertion preserves
Persian/English Unicode, punctuation and cursor/selection behavior; system
subtitles, drafts, passwords, read-only targets and repeated actions do not
produce input. Cancel/stop/clear supersede pending insertion. Preserve previous
installer and test evidence until the replacement is verified.

Android reuses the portable resolver but does not expose these new settings or
OS input injection yet. No TTS or mouse/OCR capture is included.
