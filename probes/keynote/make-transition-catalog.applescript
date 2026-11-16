-- Builds the Keynote transition catalog deck: one slide per `transition effect` the dictionary offers, each titled
-- with the effect it carries (our own text), then saves it and exports PPTX, an "all stages" PDF and a QuickTime
-- movie. Attended: run it on the Mac with Keynote 14.5 (see RUNBOOK.md); it never touches an existing document.
--
--   osascript make-transition-catalog.applescript <out dir>
--
-- Writes <out dir>/transition-catalog.key, .pptx, .pdf, .m4v and transition-catalog.tsv (slide, requested effect,
-- the effect Keynote reports back, status). Effects the dictionary does not know are logged as "unknown term" and
-- left out, so the list can hold names from several Keynote versions.

property effectNames : {"magic move", "dissolve", "fade through color", "fade and move", "move in", "push", "reveal", ¬
	"wipe", "iris", "scale", "switch", "swap", "swing", "swoosh", "twirl", "twist", "cube", "doorway", "drop", ¬
	"droplet", "fall", "flip", "flop", "grid", "mosaic", "page flip", "perspective", "pivot", "reflection", ¬
	"revolving door", "blinds", "clothesline", "color planes", "confetti", "shimmer", "sparkle", "object cube", ¬
	"object flip", "object pop", "object push", "object revolve", "object zoom", "anagram", "no transition effect"}

on run argv
	set outDir to item 1 of argv
	if outDir does not end with "/" then set outDir to outDir & "/"
	set report to "slide	requested	reported	status" & linefeed
	tell application id "com.apple.iWork.Keynote"
		activate
		with timeout of 1800 seconds
			set theDocument to make new document with properties {width:1920, height:1080}
			set slideNumber to 0
			repeat with effectName in effectNames
				set effectName to effectName as string
				tell theDocument
					if slideNumber is 0 then
						set theSlide to slide 1
					else
						set theSlide to make new slide
					end if
				end tell
				set slideNumber to slideNumber + 1
				try
					set object text of default title item of theSlide to "Transition: " & effectName & " (Keynote catalog, slide " & slideNumber & ")"
				end try
				try
					set object text of default body item of theSlide to "slidock probe: this slide is entered with the Keynote transition named \"" & effectName & "\", 1.5 s, on click."
				end try
				set status to "ok"
				set reported to ""
				try
					run script "tell application id \"com.apple.iWork.Keynote\" to set transition properties of slide " & slideNumber & " of front document to {transition effect:" & effectName & ", transition duration:1.5, transition delay:0, automatic transition:false}"
					set reported to (transition effect of (transition properties of theSlide)) as string
				on error errorMessage
					set status to "unknown term: " & errorMessage
				end try
				set report to report & slideNumber & tab & effectName & tab & reported & tab & status & linefeed
			end repeat
			save theDocument in (POSIX file (outDir & "transition-catalog.key"))
			export theDocument to (POSIX file (outDir & "transition-catalog.pptx")) as Microsoft PowerPoint
			export theDocument to (POSIX file (outDir & "transition-catalog.pdf")) as PDF with properties {PDF image quality:Best, skipped slides:false, all stages:true, include comments:false}
			export theDocument to (POSIX file (outDir & "transition-catalog.m4v")) as QuickTime movie with properties {movie format:format1080p}
			close theDocument saving no
		end timeout
	end tell
	set tsv to open for access (POSIX file (outDir & "transition-catalog.tsv")) with write permission
	set eof of tsv to 0
	write report to tsv as «class utf8»
	close access tsv
	return report
end run
