-- Exports a Keynote deck as a PDF (each slide built) and as a movie of native size, H.264, 60 fps (the motion lane's
-- transition oracle), then closes it unsaved. Keynote 14.5 shows each slide 5 s.
--   osascript export-keynote-movie.applescript <deck.key> <out.pdf> <out.m4v>
on run argv
	set deckPath to item 1 of argv
	tell application id "com.apple.iWork.Keynote"
		with timeout of 1800 seconds
			set d to open (POSIX file deckPath as alias)
			export d to (POSIX file (item 2 of argv)) as PDF with properties {PDF image quality:Best, skipped slides:false, all stages:false, include comments:false}
			export d to (POSIX file (item 3 of argv)) as QuickTime movie with properties {movie format:native size, movie codec:h264, movie framerate:FPS60}
			set w to width of d
			set h to height of d
			close d saving no
		end timeout
	end tell
	return "exported " & w & "x" & h
end run
