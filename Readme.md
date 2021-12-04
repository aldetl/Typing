Table of Contents

[TOC]

---



# Version

v1.3.0, 2021-12-02,



# Introduction

**Typing** controls cursor movements and execute some commands by typing a single character. Its purpose is to improve your typing efficiency by reducing the frequency of switching between keyboard and mouse as well as the movement distance of hands. In short, it keeps your hands on the keyboards!



# Quick start

After running, it is minimized and shows an **icon** in system **task tray** (on the taskbar).



It now supports two operation modes:

- vim-mode. 
  - Press the mode-key (default is '**CapsLock**') to enter this mode. 
  - Then, press other keys to execute commands.
- emacs-mode (hotkey)
  - Press a hotkey to execute commands. No need to enter the vim-mode. 



The basic commands in vim-mode are:

- I, K, J, L          => Up, Down, Left, Right
- N, M, U, O    => PageUp, PageDown, Home, End
- A, S, D, F       => set repeat number (10, 5, 3, 1) for next movement (I, K, J, L, N, M)
- Q                   => undo last movement (for I, K, J, L, N, M)
- T, Y                => type current time, date
- G                   => exit command-mode

If you want to send the `CapsLock` to the system, press:

- `RShiftKey+CapsLock`



Indication of current mode status:

- A highlight tip will show following the mouse cursor according to the mode status.
- The icon in system task tray will change too.



# Reference

## Vim-mode

### Mode change

| operation          | key                 | note                   |
| ------------------ | ------------------- | ---------------------- |
| enter command mode | `mode key`          | Default is `Caps Lock` |
| exit command mode  | `mode key` , or `G` |                        |

The default `mode key` is `Caps Lock`.

You can change the `mode key` in the `configuration.ini` in the folder of the program.



### movement commands

In command mode, you can press a single character to directly control cursor move. 

| key  | movement   | note |
| ---- | ---------- | ---- |
| `I`  | `Up`       |      |
| `K`  | `Down`     |      |
| `J`  | `Left`     |      |
| `L`  | `Right`    |      |
| `U`  | `Home`     |      |
| `O`  | `End`      |      |
| `N`  | `PageUp`   |      |
| `N`  | `PageDown` |      |

You can also set a step (repeat number) for next movement (I, K, J, L, N, M). The default step is 1.

| key  | movement step | note |
| ---- | ------------- | ---- |
| `A`  | 1             |      |
| `S`  | 3             |      |
| `D`  | 5             |      |
| `F`  | 10            |      |



### Other commands

| def  | name                | des  |
| ---- | ------------------- | ---- |
| `E`  | `Del`               |      |
| `R`  | `Backspace`         |      |
| `Y`  | type current Date   |      |
| `T`  | type current time   |      |
| `Q`  | under last movement |      |



## Other setting

### Keep icon showing in the task tray in Windows 10

Open "**taskbar setting**"

- right-click on the **taskbar** \ taskbar settings

then 

- click "**select which icons appear on the taskbar**" (under Notification area).
- find "**Typing**" and turn it On.



# Contact

Author email, freelssoft@gmail.com

