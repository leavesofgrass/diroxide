@echo off
:: diro: name = "Make backup copy"
:: diro: selection = "many"
:: diro: mode = "run"
:: diro: confirm = "Copy the marked files?"
copy %1 %1.bak
