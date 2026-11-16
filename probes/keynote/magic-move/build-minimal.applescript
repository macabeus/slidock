-- Builds a 4-slide Magic Move probe: an off-slide panel slides in (1→2), a matched box goes from 50 % to full opacity
-- (2→3), the panel slides back off the slide (3→4); text lines move; one picture swaps for another, one stays.
-- Usage: osascript build-minimal.applescript <out.key> <pic-a.png> <pic-b.png>
on placeShape(sl, px, py, w, h, txt, op)
	tell application id "com.apple.iWork.Keynote"
		tell sl
			set shp to make new shape with properties {position:{px, py}, width:w, height:h, opacity:op}
			if txt is not "" then set object text of shp to txt
		end tell
	end tell
	return shp
end placeShape

on shiftPanel(sl, dx)
	tell application id "com.apple.iWork.Keynote"
		tell sl
			repeat with shp in shapes
				set p to position of shp
				if (item 1 of p) > 500 then set position of shp to {(item 1 of p) + dx, item 2 of p}
			end repeat
			repeat with ln in lines
				set sp to start point of ln
				set ep to end point of ln
				if (item 1 of sp) > 500 then
					set start point of ln to {(item 1 of sp) + dx, item 2 of sp}
					set end point of ln to {(item 1 of ep) + dx, item 2 of ep}
				end if
			end repeat
		end tell
	end tell
end shiftPanel

on run argv
	set outPath to item 1 of argv
	set picA to POSIX file (item 2 of argv) as alias
	set picB to POSIX file (item 3 of argv) as alias
	tell application id "com.apple.iWork.Keynote"
		with timeout of 600 seconds
			set d to make new document with properties {document theme:theme "Basic White", width:1024, height:768}
			set s1 to slide 1 of d
			set base layout of s1 to slide layout "Blank" of d
			tell s1
				make new text item with properties {object text:"Line one", position:{20, 80}}
				make new text item with properties {object text:"Line two", position:{20, 120}}
				make new image with properties {file:picA, position:{100, 180}, width:600, height:400}
			end tell
			-- the panel, off the slide to the right (the slide is 1024 wide)
			my placeShape(s1, 1080, 0, 500, 768, "", 30)
			my placeShape(s1, 1130, 60, 340, 64, "Source code", 100)
			my placeShape(s1, 1130, 300, 340, 64, "Parser", 100)
			my placeShape(s1, 1130, 560, 340, 64, "Code generation", 50)
			tell s1
				make new line with properties {start point:{1300, 130}, end point:{1300, 295}}
				make new line with properties {start point:{1300, 370}, end point:{1300, 555}}
			end tell
			-- 2: the panel slides in, the text moves up, the picture swaps
			duplicate s1 to after s1
			set s2 to slide 2 of d
			my shiftPanel(s2, -560)
			tell s2
				repeat with t in text items
					set p to position of t
					set position of t to {item 1 of p, (item 2 of p) - 30}
				end repeat
				delete every image
				make new image with properties {file:picB, position:{40, 160}, width:300, height:500}
			end tell
			-- 3: the faded box becomes opaque
			duplicate s2 to after s2
			set s3 to slide 3 of d
			tell s3
				repeat with shp in shapes
					if (object text of shp as text) is "Code generation" then set opacity of shp to 100
				end repeat
			end tell
			-- 4: the panel slides back off the slide
			duplicate s3 to after s3
			set s4 to slide 4 of d
			my shiftPanel(s4, 560)
			repeat with sl in {s1, s2, s3}
				set transition properties of sl to {transition effect:magic move, transition duration:2.0, transition delay:0, automatic transition:false}
			end repeat
			set n to count of slides of d
			save d in (POSIX file outPath)
			close d saving no
		end timeout
	end tell
	return "built " & n & " slides"
end run
