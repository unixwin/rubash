use super::*;

pub(in crate::executor) fn apply_inherited_stderr_to_stdout_fd_copy(
    command: &mut CommandNode,
    redirect: &Redirect,
) {
    if redirect_target_fd(&redirect.target).is_some() {
        return;
    }

    if command
        .redirect_out
        .as_ref()
        .is_some_and(|redirect| redirect_target_fd(&redirect.target) == Some(2))
    {
        command.redirect_out = None;
        command.append = Some(redirect.clone());
    }

    if command
        .append
        .as_ref()
        .is_some_and(|redirect| redirect_target_fd(&redirect.target) == Some(2))
    {
        command.append = Some(redirect.clone());
    }
}
