-- Exports a hand-authored Keynote catalog deck (the build catalog of RUNBOOK.md) the way the transition catalog is
-- exported: PPTX (Apple's own mapping onto p:timing), an "all stages" PDF (one page per build stage, the bench's
-- stage oracle) and a QuickTime movie (the timing oracle). Opens the deck read-only and closes it unsaved.
--
--   osascript export-catalog.applescript <deck.key> <out dir>

on run argv
	set deckPath to item 1 of argv
	set outDir to item 2 of argv
	if outDir does not end with "/" then set outDir to outDir & "/"
	set baseName to do shell script "basename " & quoted form of deckPath & " .key"
	tell application id "com.apple.iWork.Keynote"
		with timeout of 1800 seconds
			set theDocument to open ((POSIX file deckPath) as alias)
			export theDocument to (POSIX file (outDir & baseName & ".pptx")) as Microsoft PowerPoint
			export theDocument to (POSIX file (outDir & baseName & ".pdf")) as PDF with properties {PDF image quality:Best, skipped slides:false, all stages:true, include comments:false}
			export theDocument to (POSIX file (outDir & baseName & ".m4v")) as QuickTime movie with properties {movie format:format1080p}
			close theDocument saving no
		end timeout
	end tell
end run
