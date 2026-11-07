# Photos

This guide covers the photo helpers: four columns that say when a picture was taken, with what, how big and of what kind, a sort by the date taken, Next day and Previous day, three ways to select pictures, a review that goes through pictures one key at a time, and a command that moves pictures into folders named by month.

The short version: the helpers work on the pictures in the folder a panel shows. They read each picture's own date and camera from the details the camera wrote inside the file, they never draw a picture, and every change they make is a job that Ctrl+Z takes back. A picture with no date taken goes by its modified time, and the helpers say so each time.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells, on pictures the tests made. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## What counts as a picture

A picture is a file whose type is in the Images group ([File types and groups](file-types.md)): a PNG, a JPEG, a camera's raw file and the rest of that group. A folder is not a picture and neither is a text file. In a folder with none, a helper says "No pictures here".

The helpers read each picture's date taken and camera from its Exif block, with the metadata readers of [the details view](details.md), and they never decode a pixel. They read a panel's pictures once, when a helper first needs them, and again when the folder changes. The first command says "Reading 6 pictures" while it does.

## The photo columns

Four keys in the `columns` setting add a cell to every picture's row. [Columns, notes and fields](columns.md) says how to choose columns. The photo ones are:

- `photo.taken`: when it was taken, "taken 2026-10-12 14:03". A picture with no date taken says its modified time instead, with the reason after it: "taken 2026-10-13 10:00 (modified)".
- `photo.camera`: the camera's maker and model, "camera Google Pixel 8". The maker is left out when the model already says it.
- `photo.pixels`: the size as shown, after the picture's orientation: "pixels 4032 by 3024".
- `photo.kind`: "kind photo" when a camera wrote the file, "kind screenshot" when the file's type is in the Screenshots group, and "kind picture" for any other picture. A text file has no kind cell.

A cell that is not read yet says so in words, as the other columns do, and the cells arrive for the rows near the focus first.

### The date taken, and the fallback

A date taken is the date and time the camera wrote, as its clock showed them, with no time zone. A camera with no clock writes zeros, and zeros are not a date. A picture with no date taken uses its modified time in your local time, and the fallback is said wherever it matters:

- In the Taken cell, with "(modified)" after it.
- After a sort by date taken: "Date taken; 1 uses modified time." The verb agrees with the count, so twelve say "Date taken; 12 use modified time."
- In the month folders' plan, as the last two rows: "Date taken; 1 uses modified time." and "No date taken: using modified time".

## Sort by date taken

Sort by date taken is in the View menu, in the Sort submenu, and in the palette. It sorts the pictures by the moment each was taken, the modified time standing in where there is none, and says "Sorted by date taken, oldest first." Run it again and it says "newest first". When some pictures use their modified time, the same line goes on with the note: "Sorted by date taken, oldest first. Date taken; 1 uses modified time."

## Next day and Previous day

Next day and Previous day are in the Go menu (Alt+G, then X or Y) and in the palette. They move the focus to the first picture of the next date, or of the date before, in the list's order, going by the date taken or else the modified time. They say the date, the number of pictures that day and the weekday, with the weekday last so a long date still has its count in the first 40 cells:

- "Next day: October 12, 3 pictures, Monday".
- "Next day: October 13, 1 picture, Tuesday".
- "Previous day: December 31, 2025, 1 picture, Wednesday". The year is said only when it is not this year.

The weekday is worked out from the date, not stored. Past the last date, "No later day here". Before the first, "No earlier day here".

## Select the screenshots, the bursts and the pictures with no date

The three selections are in the Select menu and the palette. Each replaces the marks you had, and keeps them for Restore selection ([The panels](panels.md)). Each says the count first and then how many are selected:

- Select the screenshots (Alt+S, then O) marks the files of the Screenshots group: "Selected 1 screenshot. 1 selected." With none: "No screenshots here".
- Select the bursts (Alt+S, then B) marks pictures taken by one camera, each within a short gap of the one before. A single burst says itself: "Burst, 3 pictures in 2 seconds. 3 selected." Several say "Selected 2 bursts, 7 pictures. 7 selected." With none: "No bursts here: no pictures from one camera within 2 seconds".
- Select pictures with no date (Alt+S, then T) marks the pictures whose own details hold no date taken: "Selected 1 picture with no date taken. 1 selected." When every picture has one, "Every picture here has a date taken".

A burst needs a camera and a date taken, because it is made of pictures one camera took close together. The gap is the Burst gap setting, `photos.burst_gap_seconds`: 1 to 60 seconds, 2 to start with. [The settings reference](settings-reference.md) lists it. A picture with no date taken, or no camera, is never part of a burst.

## Review pictures

Review pictures is in the Tools menu (Alt+T, then R) and in the palette. It steps through the marked pictures, or through every picture in the folder when none is marked, starting at the focused one. The panel's cursor follows, so the quick view shows the picture being asked about.

The list's title says where you are, the picture and what kind it is: "Review: 2 of 6, IMG_0002.jpg, photo". The rows are the answers, each with its key after its name:

- "Keep, K" leaves the picture and goes on.
- "Label, L" asks for a label in a field, "Set label on IMG_0002.jpg", with the labels you have to complete. The review stays on the same picture, so after the label press K or S to go on.
- "Move to a place you pick, M" asks where in a field, "Move IMG_0002.jpg to". The field starts with the place you last moved a picture to, or the other panel's folder, and Tab completes places. The picture moves as a job, and the review goes on.
- "Trash, T" sends the picture to the trash as a job, and the review goes on.
- "Skip, S" goes on without an answer.
- "Back, B" goes back one picture. The title then says what you answered there: "Review: 3 of 6, IMG_0003.jpg, photo, in the trash". On the first picture it says "This is the first picture".

You can press a key or move to a row and press Enter. F1 says the keys. Escape ends the review and says what was done: "Review ended at 3 of 6, 1 kept, 1 in the trash". The tally uses the words kept, labeled, moved, in the trash and skipped. Going through the last picture ends the review too, as "Review done: 6 pictures, 1 kept, 1 in the trash".

The moves and the trash are the same jobs as anywhere else, so Ctrl+Z puts the last one back, and again for the one before.

## Move into month folders

Move into month folders is in the Tools menu (Alt+T, then M) and in the palette. It moves the marked pictures, or every picture in the folder, into folders named by year and month, such as `2026-10`, from the date taken or else the modified time. A text file, a folder and anything that is not a picture stays where it is.

It asks first, and the question starts on No: "Move 6 pictures into 2 month folders? No is the default. y or n". The plan is listed under it, a row for each month, and the fallback note when it applies:

- "2025-12: 1 picture, a new folder"
- "2026-10: 5 pictures, a new folder". A month folder that already exists says no "a new folder".
- "Date taken; 1 uses modified time."
- "No date taken: using modified time"

No says "Nothing moved." Yes runs it as one job with one result, and Ctrl+Z puts every picture back and removes the folders the job made. The month folders are made where they are missing, and nothing is moved over a file. If a file with a month's name is in the way, the command stops before anything is moved: "Cannot make 2026-10: a file has that name. Nothing moved." A picture whose name is taken inside its month folder is handled by the usual conflict question ([Operations and undo](operations.md)).

## Known limits

- Bursts need a camera and a date taken in the picture's own details. The date is read to the second from the Exif block for bursts, while the Taken column is to the minute.
- Review's Label keeps the same picture, so one more key goes on.
- The month folders' moves are planned by the program and run by the job engine, so a plugin's before-move hook does not run for them.
- No picture is drawn and there are no thumbnails; a picture grid comes in a later alpha.
- The helpers read the pictures of one folder. They do not go into subfolders.
- Real camera files were not tried: the tests use small pictures made at test time, with their dates and cameras written for the test.
- Nothing about how the review's list and the day lines read on a Braille display has been heard yet.

## See also

- [Columns, notes and fields](columns.md)
- [The details view](details.md)
- [Previews and the quick view's formats](previews.md)
- [File types and groups](file-types.md)
- [The panels](panels.md)
- [Operations and undo](operations.md)
- [Labels and bookmarks](labels-bookmarks.md)
- [Settings reference](settings-reference.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
