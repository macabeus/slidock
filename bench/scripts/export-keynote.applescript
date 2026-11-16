-- Exports a Keynote presentation to PDF, every slide at its final build state (or, with `all-stages`, one page per
-- build stage), and closes the presentation unsaved.
--   osascript export-keynote.applescript <bundle id> <presentation.key> <out.pdf> [all-stages]
-- The bundle id picks the app (Keynote 14.5 is com.apple.iWork.Keynote). Without the explicit timeout an Apple Event
-- fails after 120 s while Keynote keeps working, which a large presentation exceeds. Keynote opens an alias; given a plain
-- POSIX file it returns no document.

on exportPresentation(keynoteApp, inPath, outPath, allStages)
	using terms from application id "com.apple.iWork.Keynote"
		tell keynoteApp
			with timeout of 1800 seconds
				set theDocument to open ((POSIX file inPath) as alias)
				export theDocument to (POSIX file outPath) as PDF with properties {PDF image quality:Best, skipped slides:false, all stages:allStages, include comments:false}
				try
					close theDocument saving no
				end try
			end timeout
		end tell
	end using terms from
end exportPresentation

on run argv
	set bundleId to item 1 of argv
	set allStages to false
	if (count of argv) > 3 then
		if item 4 of argv is "all-stages" then set allStages to true
	end if
	my exportPresentation(application id bundleId, item 2 of argv, item 3 of argv, allStages)
end run
