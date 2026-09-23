//! The sheet's right-click menu: what can be done to the part, the wire or
//! the pin under the pointer — or to the view, while the board runs.

use super::*;

/// The menu the last right-click opened, where it opened.
#[component]
pub(super) fn SheetMenu(board: Board) -> impl IntoView {
    let Board {
        state,
        live,
        parts,
        wires,
        selected_wire,
        menu,
        marked,
        view,
        no_connect,
        history,
        future,
        ..
    } = board;
    view! {
        {move || {
            let (x, y, target) = menu.get()?;
            let close = Callback::new(move |_| menu.set(None));
            // A running board's menu is the view's, whatever it
            // was opened on: nothing in it edits.
            let target = if live.get_untracked() { MenuTarget::Sheet } else { target };
            let items = match target {
                MenuTarget::Wire(index) => view! {
                    <MenuItem
                        label=t!("simulate.straighten")
                        on_select=Callback::new(move |_| {
                            board.straighten_wire(index);
                            menu.set(None);
                        })
                    />
                    <MenuSeparator />
                    <MenuItem
                        label=t!("simulate.disconnect")
                        shortcut="Del"
                        danger=true
                        on_select=Callback::new(move |_| {
                            board.remove_wire(index);
                            menu.set(None);
                        })
                    />
                }
                    .into_any(),
                MenuTarget::Part(index) => {
                    let (is_kit, has_wires) = parts.with_untracked(|list| {
                        let part = list.get(index);
                        let is_kit = part.is_some_and(|p| p.is_kit());
                        let reference = part.map(|p| p.inst.reference.clone()).unwrap_or_default();
                        let has_wires = wires.with_untracked(|all| {
                            all.iter().any(|w| w.from.part == reference || w.to.part == reference)
                        });
                        (is_kit, has_wires)
                    });
                    view! {
                        <MenuItem
                            label=t!("simulate.rotate")
                            shortcut="Space"
                            on_select=Callback::new(move |_| {
                                board.rotate_part(index);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label=t!("simulate.mirror")
                            shortcut="X"
                            on_select=Callback::new(move |_| {
                                board.mirror_part(index);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label=t!("simulate.duplicate")
                            shortcut="Ctrl+D"
                            disabled=is_kit
                            on_select=Callback::new(move |_| {
                                board.duplicate_part(index);
                                menu.set(None);
                            })
                        />
                        <MenuItem
                            label=t!("simulate.disconnect-wires")
                            disabled=!has_wires
                            on_select=Callback::new(move |_| {
                                board.disconnect_all(index);
                                menu.set(None);
                            })
                        />
                        <MenuSeparator />
                        <MenuItem
                            label=t!("simulate.remove")
                            shortcut="Del"
                            danger=true
                            disabled=is_kit
                            on_select=Callback::new(move |_| {
                                board.remove_part(index);
                                menu.set(None);
                            })
                        />
                    }
                        .into_any()
                }
                MenuTarget::Pin(part, pin) => {
                    let marked_already = parts.with_untracked(|list| {
                        list.get(part)
                            .and_then(|p| p.symbol.as_ref())
                            .and_then(|s| s.pins.get(pin).map(|f| (s, f)))
                            .zip(list.get(part))
                            .is_some_and(|((symbol, found), owner)| {
                                let named = PinRef::new(
                                    &owner.inst.reference,
                                    symbol.wire_key(found),
                                );
                                no_connect.with_untracked(|m| m.contains(&named))
                            })
                    });
                    let label = if marked_already {
                        t!("simulate.connected-again")
                    } else {
                        t!("simulate.not-connected")
                    };
                    view! {
                        <MenuItem
                            label=label
                            on_select=Callback::new(move |_| {
                                board.toggle_no_connect(part, pin);
                                menu.set(None);
                            })
                        />
                    }
                    .into_any()
                }
                MenuTarget::Sheet => view! {
                    {(!live.get_untracked())
                        .then(|| {
                            view! {
                                <MenuItem
                                    label=t!("menu.edit.undo")
                                    shortcut="Ctrl+Z"
                                    disabled=history.with_untracked(Vec::is_empty)
                                    on_select=Callback::new(move |_| {
                                        board.undo();
                                        menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("menu.edit.redo")
                                    shortcut="Ctrl+Y"
                                    disabled=future.with_untracked(Vec::is_empty)
                                    on_select=Callback::new(move |_| {
                                        board.redo();
                                        menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("simulate.select-all")
                                    shortcut="Ctrl+A"
                                    on_select=Callback::new(move |_| {
                                        marked.set((0..parts.with_untracked(Vec::len)).collect());
                                        selected_wire.set(None);
                                        menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                            }
                        })}
                    {state
                        .sim.plan
                        .with_untracked(|plan| {
                            plan.as_ref()
                                .and_then(|p| p.debug.as_ref())
                                .map(|d| d.gdb_command.clone())
                        })
                        .map(|command| {
                            view! {
                                <MenuItem
                                    label=t!("simulate.open-gdb")
                                    on_select=Callback::new(move |_| {
                                        controller::attach_debugger_terminal(
                                            state,
                                            command.clone(),
                                        );
                                        menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                            }
                        })}
                    <MenuItem
                        label=t!("simulate.fit-contents")
                        shortcut="F"
                        on_select=Callback::new(move |_| {
                            board.fit_view();
                            menu.set(None);
                        })
                    />
                    <MenuItem
                        label=t!("simulate.reset-view")
                        shortcut="1:1"
                        on_select=Callback::new(move |_| {
                            view.set((0.0, 0.0, 1.0));
                            menu.set(None);
                        })
                    />
                }
                    .into_any(),
            };
            Some(view! { <ContextMenu x=x y=y on_close=close>{items}</ContextMenu> })
        }}
    }
}
