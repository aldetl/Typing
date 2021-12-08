Table of Contents

[TOC]

---



v1.3.2



# Introduction

**Typing** is a tool to improve your typing efficiency by reducing the frequency of switching between keyboard and mouse as well as the movement distance of hands. In short, it keeps your hands on the keyboards.

In **Typing**, you can move cursor by pressing character keys (vim-like movement) or by pressing hotkeys (emacs-like movement). You can also define and send customized hotkeys.



# Quick start

After running, it is minimized into system **task tray** (on the taskbar).

It now supports two operation modes:

- vim-mode (command-mode)
  - Press the mode-key (default is '**CapsLock**') to enter this mode. 
  - Then, press other keys to execute commands.
- emacs-mode (hotkey-mode)
  - Press a hotkey to execute commands. No need to enter the vim-mode. 

The basic commands in command-mode are:

- `I, K, J, L`          => `Up, Down, Left, Right`
- `N, M, U, O`    => `PageUp, PageDown, Home, End`
- `A, S, D, F`       => set repeat number (10, 5, 2, 1) for next move (`I, K, J, L, N, M`)
- `Q `                  => undo last move (`I, K, J, L, N, M`)
- `T, Y`                => type current time, date
- `G`                   => exit command-mode

- `RShiftKey+CapsLock`  => `CapsLock`

Notification of current mode state:

- A highlight tip will show following the mouse cursor according to the mode status.
- The icon in system task tray will change too.



# Settings

Settings are defined in `configuration.ini` in the folder of the program.



## Mode change

| key                 | operation          | note |
| ------------------- | ------------------ | ---- |
| `CapsLock`          | enter command mode |      |
| `CapsLock` , or `G` | exit command mode  |      |

The default `mode-key` is `CapsLock`.



## Movement commands

In command mode, you can press a single character to directly control cursor move. 

| command-mode | hotkey-mode  | movement   |
| ------------ | ------------ | ---------- |
| `I`          | `CapsLock+I` | `Up`       |
| `K`          | `CapsLock+K` | `Down`     |
| `J`          | `CapsLock+J` | `Left`     |
| `L`          | `CapsLock+L` | `Right`    |
| `U`          | `CapsLock+U` | `Home`     |
| `O`          | `CapsLock+O` | `End`      |
| `N`          | `CapsLock+N` | `PageUp`   |
| `M`          | `CapsLock+M` | `PageDown` |

You can also set a step (repeat number) for next movement (`I, K, J, L, N, M`). The default step is 1.

| key  | movement step | note |
| ---- | ------------- | ---- |
| `A`  | 10            |      |
| `S`  | 5             |      |
| `D`  | 2             |      |
| `F`  | 1             |      |



## Other commands

| command-mode | hotkey-mode  | name                |
| ------------ | ------------ | ------------------- |
| `E`          | `CapsLock+E` | `Del`               |
| `R`          | `CapsLock+R` | `Backspace`         |
| `Y`          | `CapsLock+Y` | type current Date   |
| `T`          | `CapsLock+T` | type current time   |
| `Q`          | `CapsLock+Q` | under last movement |



## Text selection

Test selection is achieved by sending `Shift` modifier key. There are three ways to do this.

- Enter command-mode; Type `V`; Select using movement keys.
- Enter command-mode; Select using `Shift+`movement keys.
- Any state, Select using hotkey `CapsLock+Shift+`movement keys.

Note

- The first method supports the step setting, while other two method not.



## State notification

The program can show whether the it is in command-mode or not. It can also show the state of the `CapsLock` key.

There are two ways to show the states.

- A circle will be drawn at the position of the mouse cursor. The color and radius can be set in the configuration file.
- The icon in task tray changes according to current state. 



# Other setting

## Keep icon showing in the task tray in Windows 10

Open "**taskbar setting**"

- right-click on the **taskbar** \ taskbar settings

then 

- click "**select which icons appear on the taskbar**" (under Notification area).
- find "**Typing**" and turn it On.



# Contact

Author email, freelssoft@gmail.com

