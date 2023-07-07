Table of contents

[TOC]

---



## v1.3.2.4

2023-07-06,

- Use hotkey to enable/disable the program. The hotkey is defined in 'configuration.ini'.



## v1.3.2.3

2023-07-05

- Double click notify tray icon to enable/disable the program. 



## v1.3.2.2

2021-12-16,

- Fix bug: mode-key does not work sometimes.

- Hotkey commands will not affect time counting of mode auto-exit.

- .net framework 4.8.



## v1.3.2.1

2021-12-07,

- fix bug: color setting for mode tip.



## v1.3.2

2021-12-06, 


- improve performance.
- `LControlKey, RControlKey, LMenu, RMenu` can be set on the right side now.



## v1.3.1

2021-12-03, 

- Shift section works now；

Bug：

- （2021-12-03）`LControlKey, RControlKey, LMenu, RMenu` on the right side of settings cannot be recognized correctly. But `Control, Alt, LShiftKey, RShiftKey, Shift` work fine.



## v1.3.0

2021-12-02， 

+ show `CapsLock` state;
+ support text selection by sending `Shift` modifier key;

There are three ways to select text:

- Enter `command-mode`，press `V` to enable selection function, then select text by using movement keys；

- Enter `command-mode`, press `Shift+ movement-key` to do selection;

- Under any condition, press selection hotkeys. e.g., 

  - `CapsLock+LShiftKey+U=RShiftKey+Home`, 

  - or `LMenu+U=RShiftKey+Home` (need to enable in the configuration file)

Note：

- The first way support step commands, while others do not;
- The selection function is achieved by sending `Shift` modifier key. However, the software does not add `Shift` to the keys you want to send even the `Shift` key is pressed. All the commands need to be listed in the configuration file. In this way, the `Shift` key can be customized to achieve any possible combination.

Bug

- Selection function does not work in some systems.



## v1.2.0

2021-10-22,

- Support exit the `command-mode` automatically after a time delay. 
- Support show the state of the `command-mode`, by showing a circle following the mouse cursor. 



## v1.1.0

2021-10-20,

- Support two ways to send a command:
  - Enter `command-mode`, press a character key (`vim-style`).
  - Send hotkey directly (`emacs-style`).
- Support define your own commands in the `configuration.ini`.
- Remove the function of change `command-mode` by double pressing the `mode-key`.



## v1.0.2.1

2021-10-14, 

- fix bug：1.0.2 high cpu usage

## v1.0.2

2021-10-14, 

- fix bug: press the mode-key twice continuously.
- support emacs-style hotkeys.



## v1.0. 1

2021-10-12，

- custom setting in `configuration.ini`



## v1.0.0

2021-10-11， 
