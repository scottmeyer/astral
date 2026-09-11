//! Searchable, bounded terminal presentation; choices are inert controller data.
use std::io;

#[derive(Debug, Clone)]
pub struct Choice {
    pub label: String,
    pub detail: Vec<String>,
    pub searchable: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Chosen(usize),
    Back,
    Cancel,
}

/// Owns raw mode and the alternate screen. Drop this before launching a child.
/// Normal unwinding restores the terminal without replacing global panic hooks.
pub struct Terminal {
    inner: backend::Screen,
}

pub fn available() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

impl Terminal {
    pub fn open() -> io::Result<Self> {
        backend::Screen::open().map(|inner| Self { inner })
    }

    /// Each view starts a new query. Returned indices refer to the original list.
    pub fn select(
        &mut self,
        title: &str,
        choices: &[Choice],
        initial: usize,
    ) -> io::Result<Selection> {
        self.inner.select(title, choices, initial)
    }

    /// Final review has fixed action rows and fully wrapped, pageable details.
    pub fn review(
        &mut self,
        title: &str,
        choices: &[Choice],
        initial: usize,
    ) -> io::Result<Selection> {
        self.inner.review(title, choices, initial)
    }
}

mod backend {
    use super::*;
    use crossterm::{
        Command,
        cursor::{Hide, MoveTo, Show},
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
        style::{Attribute, Print, ResetColor, SetAttribute},
        terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use std::{
        io::{Stdout, Write},
        marker::PhantomData,
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
    };

    const MAX_CHOICES: usize = 8192;
    const MAX_DETAILS: usize = 16;
    const MAX_FIELD_BYTES: usize = 16_384;
    const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
    const MAX_QUERY_CHARS: usize = 256;
    const MIN_COLUMNS: u16 = 20;
    const MIN_ROWS: u16 = 6;
    static ACTIVE: AtomicBool = AtomicBool::new(false);

    pub(super) struct Screen {
        stdout: Stdout,
        active: bool,
        // Crossterm's event reader must stay on one thread.
        _same_thread: PhantomData<Rc<()>>,
    }

    impl Screen {
        pub(super) fn open() -> io::Result<Self> {
            if !available() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "interactive picker requires terminal stdin/stdout and TERM other than dumb",
                ));
            }
            #[cfg(windows)]
            if !crossterm::ansi_support::supports_ansi() {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "interactive picker requires an ANSI-capable Windows terminal",
                ));
            }
            if ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "a terminal picker is already active",
                ));
            }
            let mut screen = Self {
                stdout: io::stdout(),
                active: false,
                _same_thread: PhantomData,
            };
            match terminal::is_raw_mode_enabled() {
                Ok(false) => (),
                Ok(true) => {
                    ACTIVE.store(false, Ordering::Release);
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "terminal raw mode is already owned",
                    ));
                }
                Err(error) => {
                    ACTIVE.store(false, Ordering::Release);
                    return Err(error);
                }
            }
            if let Err(error) = terminal::enable_raw_mode() {
                ACTIVE.store(false, Ordering::Release);
                return Err(error);
            }
            // Arm restoration before the first escape sequence: a partial write
            // still needs raw-mode cleanup and an attempted main-screen restore.
            screen.active = true;
            execute!(
                screen.stdout,
                EnterAlternateScreen,
                Hide,
                ResetColor,
                SetAttribute(Attribute::Reset)
            )?;
            Ok(screen)
        }

        fn restore(&mut self) {
            if !self.active {
                return;
            }
            self.active = false;
            let _ = terminal::disable_raw_mode();
            let _ = execute!(
                self.stdout,
                SetAttribute(Attribute::Reset),
                ResetColor,
                Show,
                LeaveAlternateScreen
            );
            let _ = self.stdout.flush();
            ACTIVE.store(false, Ordering::Release);
        }

        pub(super) fn select(
            &mut self,
            title: &str,
            choices: &[Choice],
            initial: usize,
        ) -> io::Result<Selection> {
            if !self.active {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "terminal picker is closed",
                ));
            }
            let result = self.select_inner(title, choices, initial);
            if result.is_err() {
                self.restore();
            }
            result
        }

        pub(super) fn review(
            &mut self,
            title: &str,
            choices: &[Choice],
            initial: usize,
        ) -> io::Result<Selection> {
            if !self.active {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "terminal picker is closed",
                ));
            }
            let result = self.review_inner(title, choices, initial);
            if result.is_err() {
                self.restore();
            }
            result
        }

        fn review_inner(
            &mut self,
            title: &str,
            choices: &[Choice],
            initial: usize,
        ) -> io::Result<Selection> {
            let prepared = prepare_review(title, choices)?;
            let title = sanitize(title);
            let mut cursor = initial.min(prepared.len().saturating_sub(1));
            let mut offset = 0;
            let mut size = terminal::size()?;
            loop {
                let (frame, page, total) =
                    review_frame(&title, &prepared, cursor, &mut offset, size)?;
                self.stdout.write_all(&frame)?;
                self.stdout.flush()?;
                loop {
                    match event::read()? {
                        Event::Resize(columns, rows) => {
                            size = (columns, rows);
                            break;
                        }
                        Event::Key(key) if key.kind != KeyEventKind::Release => {
                            if key.code == KeyCode::Esc {
                                return Ok(Selection::Back);
                            }
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && matches!(key.code, KeyCode::Char('c' | 'C'))
                            {
                                return Ok(Selection::Cancel);
                            }
                            if review_layout(size, prepared.len()).is_none() {
                                continue;
                            }
                            match key.code {
                                KeyCode::Enter if !prepared.is_empty() => {
                                    return Ok(Selection::Chosen(cursor));
                                }
                                KeyCode::Up => {
                                    cursor = cursor.saturating_sub(1);
                                    offset = 0;
                                }
                                KeyCode::Down => {
                                    cursor = cursor
                                        .saturating_add(1)
                                        .min(prepared.len().saturating_sub(1));
                                    offset = 0;
                                }
                                KeyCode::PageUp => offset = offset.saturating_sub(page),
                                KeyCode::PageDown => {
                                    offset =
                                        offset.saturating_add(page).min(total.saturating_sub(page))
                                }
                                KeyCode::Home => offset = 0,
                                KeyCode::End => offset = total.saturating_sub(page),
                                _ => continue,
                            }
                            break;
                        }
                        _ => (),
                    }
                }
            }
        }

        fn select_inner(
            &mut self,
            title: &str,
            choices: &[Choice],
            initial: usize,
        ) -> io::Result<Selection> {
            let choices = prepare(title, choices)?;
            let title = sanitize(title);
            let mut state = Search::new(choices.len(), initial);
            let mut size = terminal::size()?;
            loop {
                let frame = frame(&title, &choices, &mut state, size)?;
                self.stdout.write_all(&frame)?;
                self.stdout.flush()?;
                loop {
                    match event::read()? {
                        Event::Resize(columns, rows) => {
                            size = (columns, rows);
                            break;
                        }
                        Event::Key(key) if key.kind != KeyEventKind::Release => {
                            if key.code == KeyCode::Esc {
                                return Ok(Selection::Back);
                            }
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && matches!(key.code, KeyCode::Char('c' | 'C'))
                            {
                                return Ok(Selection::Cancel);
                            }
                            // Do not accept an invisible item on a tiny screen.
                            if size.0 < MIN_COLUMNS || size.1 < MIN_ROWS {
                                continue;
                            }
                            if key.code == KeyCode::Enter {
                                if let Some(index) = state.selected() {
                                    return Ok(Selection::Chosen(index));
                                }
                            } else if state.key(
                                key,
                                &choices,
                                list_rows(size.1.min(100), &choices, &state),
                            ) {
                                break;
                            }
                        }
                        _ => (),
                    }
                }
            }
        }
    }
    impl Drop for Screen {
        fn drop(&mut self) {
            self.restore();
        }
    }

    struct Prepared {
        label: String,
        detail: Vec<String>,
        searchable: String,
    }
    fn invalid() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "picker choices exceed the bounded display/search limits",
        )
    }
    fn prepare(title: &str, choices: &[Choice]) -> io::Result<Vec<Prepared>> {
        if title.len() > MAX_FIELD_BYTES || choices.len() > MAX_CHOICES {
            return Err(invalid());
        }
        let mut total = title.len();
        for choice in choices {
            if choice.detail.len() > MAX_DETAILS {
                return Err(invalid());
            }
            for field in std::iter::once(&choice.label)
                .chain(std::iter::once(&choice.searchable))
                .chain(choice.detail.iter())
            {
                if field.len() > MAX_FIELD_BYTES {
                    return Err(invalid());
                }
                total = total.checked_add(field.len()).ok_or_else(invalid)?;
                if total > MAX_TOTAL_BYTES {
                    return Err(invalid());
                }
            }
        }
        Ok(choices
            .iter()
            .map(|choice| Prepared {
                label: sanitize(&choice.label),
                detail: choice.detail.iter().map(|text| sanitize(text)).collect(),
                searchable: format!("{} {}", choice.label, choice.searchable).to_lowercase(),
            })
            .collect())
    }

    fn prepare_review(title: &str, choices: &[Choice]) -> io::Result<Vec<Prepared>> {
        if choices.len() > 8 {
            return Err(invalid());
        }
        let mut prepared = prepare(title, choices)?;
        // Review must not truncate fields: full paths remain available through
        // wrapping and PageUp/PageDown, within the same source-byte limits.
        for (prepared, source) in prepared.iter_mut().zip(choices) {
            prepared.detail = source
                .detail
                .iter()
                .map(|text| text.chars().flat_map(char::escape_debug).collect())
                .collect();
        }
        Ok(prepared)
    }

    /// Escape controls, bidi and other nonprinting scalars; keep visible Unicode.
    /// Truncation happens on character boundaries before any terminal write.
    fn sanitize(text: &str) -> String {
        let mut value: String = text
            .chars()
            .take(2048)
            .flat_map(char::escape_debug)
            .collect();
        if text.chars().nth(2048).is_some() {
            value.push_str("...");
        }
        value
    }
    // Conservative cell accounting avoids a unicode-width dependency: visible
    // non-ASCII scalars cost two columns, including terminals' wide glyphs.
    // This may leave space unused; it never clips a UTF-8 scalar in half.
    fn cells(c: char) -> usize {
        if c.is_ascii() { 1 } else { 2 }
    }
    fn clip(text: &str, width: usize) -> String {
        if text.chars().map(cells).sum::<usize>() <= width {
            return text.into();
        }
        if width <= 3 {
            return ".".repeat(width);
        }
        let mut used = 0;
        let mut clipped = String::new();
        for c in text.chars() {
            if used + cells(c) > width - 3 {
                break;
            }
            used += cells(c);
            clipped.push(c);
        }
        clipped.push_str("...");
        clipped
    }

    struct Search {
        query: String,
        matches: Vec<usize>,
        cursor: usize,
        offset: usize,
    }
    impl Search {
        fn new(count: usize, initial: usize) -> Self {
            Self {
                query: String::new(),
                matches: (0..count).collect(),
                cursor: initial.min(count.saturating_sub(1)),
                offset: 0,
            }
        }
        fn selected(&self) -> Option<usize> {
            self.matches.get(self.cursor).copied()
        }
        fn filter(&mut self, choices: &[Prepared]) {
            let previous = self.selected();
            let query = self.query.to_lowercase();
            let tokens: Vec<_> = query.split_whitespace().collect();
            self.matches = choices
                .iter()
                .enumerate()
                .filter_map(|(index, choice)| {
                    tokens
                        .iter()
                        .all(|token| choice.searchable.contains(token))
                        .then_some(index)
                })
                .collect();
            self.cursor = previous
                .and_then(|old| self.matches.iter().position(|index| *index == old))
                .unwrap_or(0);
            self.offset = 0;
        }
        fn key(&mut self, key: KeyEvent, choices: &[Prepared], page: usize) -> bool {
            match key.code {
                KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
                KeyCode::Down => {
                    self.cursor = self
                        .cursor
                        .saturating_add(1)
                        .min(self.matches.len().saturating_sub(1))
                }
                KeyCode::Home => self.cursor = 0,
                KeyCode::End => self.cursor = self.matches.len().saturating_sub(1),
                KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(page.max(1)),
                KeyCode::PageDown => {
                    self.cursor = self
                        .cursor
                        .saturating_add(page.max(1))
                        .min(self.matches.len().saturating_sub(1))
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.filter(choices);
                }
                KeyCode::Char('u' | 'U') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.filter(choices);
                }
                KeyCode::Char(c)
                    if !c.is_control()
                        && !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) =>
                {
                    if self.query.chars().count() < MAX_QUERY_CHARS {
                        self.query.push(c);
                        self.filter(choices);
                    }
                }
                _ => return false,
            }
            true
        }
    }

    fn wrap(text: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let mut lines = Vec::new();
        let mut current = String::new();
        let mut used = 0;
        for c in text.chars() {
            if used + cells(c) > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push(c);
            used += cells(c);
        }
        lines.push(current);
        lines
    }

    // Width, height, first action row, and detail page size. Essential details
    // can always be paged; action rows never move off the screen.
    fn review_layout(size: (u16, u16), actions: usize) -> Option<(usize, u16, u16, usize)> {
        let columns = size.0.min(240);
        let height = size.1.min(100);
        if columns < MIN_COLUMNS || height < 10 || usize::from(height) < actions + 7 {
            return None;
        }
        let action_row = height - actions as u16 - 2;
        Some((
            usize::from(columns - 1),
            height,
            action_row,
            usize::from(action_row - 3),
        ))
    }

    fn review_frame(
        title: &str,
        choices: &[Prepared],
        cursor: usize,
        offset: &mut usize,
        size: (u16, u16),
    ) -> io::Result<(Vec<u8>, usize, usize)> {
        let mut frame = blank_frame()?;
        let Some((width, height, action_row, page)) = review_layout(size, choices.len()) else {
            if size.1 > 0 {
                line(
                    &mut frame,
                    0,
                    "Resize to at least 20x10 for launch review. Esc back; Ctrl-C cancel.",
                    usize::from(size.0.min(240).saturating_sub(1)),
                    false,
                )?;
            }
            return Ok((frame, 0, 0));
        };
        let detail: Vec<_> = choices
            .get(cursor)
            .into_iter()
            .flat_map(|choice| choice.detail.iter())
            .flat_map(|text| wrap(text, width))
            .collect();
        *offset = (*offset).min(detail.len().saturating_sub(page));
        line(&mut frame, 0, title, width, false)?;
        let first = if detail.is_empty() { 0 } else { *offset + 1 };
        let last = (*offset + page).min(detail.len());
        line(
            &mut frame,
            1,
            &format!("Details {}-{} / {}", first, last, detail.len()),
            width,
            false,
        )?;
        for (row, text) in detail.iter().skip(*offset).take(page).enumerate() {
            line(&mut frame, 2 + row as u16, text, width, false)?;
        }
        line(
            &mut frame,
            action_row - 1,
            "Actions (Up/Down)",
            width,
            false,
        )?;
        for (row, choice) in choices.iter().enumerate() {
            line(
                &mut frame,
                action_row + row as u16,
                &format!(
                    "{}{}",
                    if cursor == row { "> " } else { "  " },
                    choice.label
                ),
                width,
                cursor == row,
            )?;
        }
        line(&mut frame, height - 2, "PgUp/PgDn: details", width, false)?;
        line(
            &mut frame,
            height - 1,
            "Enter choose | Esc back | Ctrl-C cancel",
            width,
            false,
        )?;
        Ok((frame, page, detail.len()))
    }

    fn list_rows(height: u16, choices: &[Prepared], state: &Search) -> usize {
        let body = usize::from(height.saturating_sub(4));
        let details = state
            .selected()
            .map_or(0, |index| choices[index].detail.len());
        let detail_rows = if body >= 4 && details > 0 {
            (details + 1).min(body / 2).min(9)
        } else {
            0
        };
        body.saturating_sub(detail_rows)
            .max(1)
            .min(state.matches.len().max(1))
    }
    // Encode without queue!'s legacy Windows WinAPI fallback. Building a frame
    // is pure; only the owned stdout write presents it to the ANSI terminal.
    fn ansi(frame: &mut Vec<u8>, command: impl Command) -> io::Result<()> {
        let mut encoded = String::new();
        command
            .write_ansi(&mut encoded)
            .map_err(|_| io::Error::other("cannot encode picker frame"))?;
        frame.extend_from_slice(encoded.as_bytes());
        Ok(())
    }
    fn blank_frame() -> io::Result<Vec<u8>> {
        let mut frame = Vec::new();
        ansi(&mut frame, MoveTo(0, 0))?;
        ansi(&mut frame, SetAttribute(Attribute::Reset))?;
        ansi(&mut frame, Clear(ClearType::All))?;
        Ok(frame)
    }

    fn line(
        frame: &mut Vec<u8>,
        row: u16,
        text: &str,
        width: usize,
        selected: bool,
    ) -> io::Result<()> {
        ansi(frame, MoveTo(0, row))?;
        ansi(frame, SetAttribute(Attribute::Reset))?;
        if selected {
            ansi(frame, SetAttribute(Attribute::Reverse))?;
        }
        ansi(frame, Print(clip(text, width)))?;
        ansi(frame, SetAttribute(Attribute::Reset))
    }
    fn frame(
        title: &str,
        choices: &[Prepared],
        state: &mut Search,
        size: (u16, u16),
    ) -> io::Result<Vec<u8>> {
        let columns = size.0.min(240);
        let height = size.1.min(100);
        let width = usize::from(columns.saturating_sub(1));
        let mut frame = blank_frame()?;
        if columns < MIN_COLUMNS || height < MIN_ROWS {
            if height > 0 {
                line(
                    &mut frame,
                    0,
                    "Resize terminal to at least 20x6. Esc back; Ctrl-C cancel.",
                    width,
                    false,
                )?;
            }
            return Ok(frame);
        }
        line(&mut frame, 0, title, width, false)?;
        line(
            &mut frame,
            1,
            &format!("Search: {}", sanitize(&state.query)),
            width,
            false,
        )?;
        line(
            &mut frame,
            2,
            &format!("{} of {} choices", state.matches.len(), choices.len()),
            width,
            false,
        )?;
        let slots = list_rows(height, choices, state);
        if state.cursor < state.offset {
            state.offset = state.cursor;
        }
        if state.cursor >= state.offset + slots {
            state.offset = state.cursor + 1 - slots;
        }
        state.offset = state.offset.min(state.matches.len().saturating_sub(slots));
        if state.matches.is_empty() {
            line(
                &mut frame,
                3,
                "No matches. Backspace to revise search.",
                width,
                false,
            )?;
        } else {
            for (row, index) in state
                .matches
                .iter()
                .skip(state.offset)
                .take(slots)
                .enumerate()
            {
                let selected = state.offset + row == state.cursor;
                line(
                    &mut frame,
                    3 + row as u16,
                    &format!(
                        "{}{}",
                        if selected { "> " } else { "  " },
                        choices[*index].label
                    ),
                    width,
                    selected,
                )?;
            }
        }
        let detail_start = 3 + slots as u16;
        if detail_start + 1 < height - 1 {
            if let Some(index) = state.selected() {
                line(&mut frame, detail_start, "Details", width, false)?;
                for (row, detail) in choices[index]
                    .detail
                    .iter()
                    .take(usize::from(height - 2 - detail_start))
                    .enumerate()
                {
                    line(
                        &mut frame,
                        detail_start + 1 + row as u16,
                        detail,
                        width,
                        false,
                    )?;
                }
            }
        }
        line(
            &mut frame,
            height - 1,
            "Type to search | Up/Down select | Enter choose | Esc back | Ctrl-C cancel",
            width,
            false,
        )?;
        Ok(frame)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn choices() -> Vec<Prepared> {
            prepare(
                "Choose",
                &[
                    Choice {
                        label: "Web".into(),
                        detail: vec!["Frontend work".into()],
                        searchable: "Authentication UI".into(),
                    },
                    Choice {
                        label: "Café".into(),
                        detail: vec![],
                        searchable: "Backend queue".into(),
                    },
                    Choice {
                        label: "API".into(),
                        detail: vec![],
                        searchable: "Authentication backend".into(),
                    },
                ],
            )
            .unwrap()
        }
        #[test]
        fn search_is_case_insensitive_token_and_and_preserves_original_indices() {
            let choices = choices();
            let mut state = Search::new(choices.len(), 0);
            state.query = "BACKEND auth".into();
            state.filter(&choices);
            assert_eq!(state.matches, [2]);
            assert_eq!(state.selected(), Some(2));
            state.query = "CAFÉ".into();
            state.filter(&choices);
            assert_eq!(state.selected(), Some(1));
            state.query = "nothing".into();
            state.filter(&choices);
            assert_eq!(state.selected(), None);
        }
        #[test]
        fn keyboard_navigation_and_unicode_backspace_stay_in_bounds() {
            let choices = choices();
            let mut state = Search::new(choices.len(), usize::MAX);
            assert_eq!(state.selected(), Some(2));
            state.key(
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                &choices,
                2,
            );
            assert_eq!(state.selected(), Some(0));
            state.key(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                &choices,
                2,
            );
            assert_eq!(state.selected(), Some(2));
            state.query = "café".into();
            state.key(
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                &choices,
                2,
            );
            assert_eq!(state.query, "caf");
            assert_eq!(state.selected(), Some(1));
            state.query = "x".repeat(MAX_QUERY_CHARS);
            state.key(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                &choices,
                2,
            );
            assert_eq!(state.query.len(), MAX_QUERY_CHARS);
        }
        #[test]
        fn sanitizing_never_emits_terminal_controls_or_bidi_controls() {
            let safe = sanitize("café\n\r\t\x1b[31m\u{202e}label");
            assert!(safe.starts_with("café\\n\\r\\t\\u{1b}[31m"));
            assert!(!safe.chars().any(char::is_control));
            assert!(!safe.contains('\u{202e}'));
        }
        #[test]
        fn clipping_respects_conservative_unicode_cells_and_tiny_widths() {
            assert_eq!(clip("abcdef", 5), "ab...");
            assert_eq!(clip("你好世界", 5), "你...");
            assert_eq!(clip("hello", 0), "");
            assert_eq!(clip("hello", 2), "..");
            for width in 0..20 {
                let clipped = clip("a你好世界éz", width);
                assert!(clipped.chars().map(cells).sum::<usize>() <= width);
            }
        }
        #[test]
        fn frame_keeps_selection_visible_and_empty_search_inert() {
            let choices = choices();
            let mut state = Search::new(choices.len(), 2);
            let output = String::from_utf8(
                frame("Choose a context", &choices, &mut state, (80, 6)).unwrap(),
            )
            .unwrap();
            assert!(output.contains("> API"));
            assert!(state.offset > 0);
            state.query = "missing".into();
            state.filter(&choices);
            let output =
                String::from_utf8(frame("Choose", &choices, &mut state, (80, 24)).unwrap())
                    .unwrap();
            assert!(output.contains("No matches. Backspace to revise search."));
            assert_eq!(state.selected(), None);
            let tiny =
                String::from_utf8(frame("Choose", &choices, &mut state, (10, 2)).unwrap()).unwrap();
            assert!(!tiny.contains("> API"));
        }
        #[test]
        fn review_blocks_confirmation_in_small_screens_and_keeps_actions_visible() {
            assert!(review_layout((20, 9), 2).is_none());
            assert!(review_layout((19, 24), 2).is_none());
            assert!(review_layout((20, 10), 2).is_some());
            let choices = prepare_review(
                "Review launch",
                &[
                    Choice {
                        label: "Launch".into(),
                        detail: vec!["Context: web".into(), "Work: none".into()],
                        searchable: String::new(),
                    },
                    Choice {
                        label: "Back".into(),
                        detail: vec!["Return to selection".into()],
                        searchable: String::new(),
                    },
                ],
            )
            .unwrap();
            let mut offset = 0;
            let (small, page, _) =
                review_frame("Review launch", &choices, 0, &mut offset, (20, 6)).unwrap();
            assert_eq!(page, 0);
            assert!(!String::from_utf8(small).unwrap().contains("> Launch"));
            let (normal, page, _) =
                review_frame("Review launch", &choices, 0, &mut offset, (80, 24)).unwrap();
            assert!(page > 0);
            let normal = String::from_utf8(normal).unwrap();
            assert!(normal.contains("Context: web"));
            assert!(normal.contains("> Launch"));
            assert!(normal.contains("PgUp/PgDn: details"));
        }

        #[test]
        fn review_pages_preserve_the_full_long_path_beyond_menu_truncation() {
            let path = format!("Worktree: /{}/Q", "a".repeat(4096));
            let choices = prepare_review(
                "Review launch",
                &[Choice {
                    label: "Launch".into(),
                    detail: vec![path.clone()],
                    searchable: String::new(),
                }],
            )
            .unwrap();
            assert_eq!(choices[0].detail[0], path);
            let wrapped = wrap(&path, 19);
            assert_eq!(wrapped.concat(), path);
            assert!(
                wrapped
                    .iter()
                    .all(|line| line.chars().map(cells).sum::<usize>() <= 19)
            );
            let mut offset = 0;
            let (first, page, total) =
                review_frame("Review launch", &choices, 0, &mut offset, (20, 10)).unwrap();
            assert!(total > page);
            assert!(!String::from_utf8(first).unwrap().contains('Q'));
            offset = total.saturating_sub(page);
            let (last, _, _) =
                review_frame("Review launch", &choices, 0, &mut offset, (20, 10)).unwrap();
            let last = String::from_utf8(last).unwrap();
            assert!(last.contains('Q'));
            assert!(last.contains("> Launch"));
        }

        #[test]
        fn oversized_fields_or_lists_fail_before_preparing_display() {
            assert!(prepare(&"x".repeat(MAX_FIELD_BYTES + 1), &[]).is_err());
            let choice = Choice {
                label: "x".into(),
                detail: vec![],
                searchable: "x".repeat(MAX_FIELD_BYTES + 1),
            };
            assert!(prepare("Choose", &[choice]).is_err());
            assert!(
                prepare(
                    "Choose",
                    &vec![
                        Choice {
                            label: String::new(),
                            detail: vec![],
                            searchable: String::new()
                        };
                        MAX_CHOICES + 1
                    ]
                )
                .is_err()
            );
        }
    }
}
