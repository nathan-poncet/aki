use super::change_request::{ChangeRequest, Checks, Note, ReviewState, Reviewer, Thread};
use super::item::{Actor, ItemRef, WorkItem};
use super::rules::{OthersDrafts, Resolver, RuleOverride, Rules};
use super::task::{Bucket, Detail, Kind, Task, Trigger};
use super::ticket::{StatusCategory, Ticket};
use super::time::Timestamp;
use super::triage::triage;

const DAY: i64 = 86_400;
const NOW: Timestamp = Timestamp(100 * DAY);

fn item() -> ItemRef {
    ItemRef {
        source: "gitlab".into(),
        container: "group/web".into(),
        reference: "!7".into(),
        title: "Add CSV export".into(),
        url: "https://example.test/mr/7".into(),
        updated_at: Timestamp(99 * DAY),
    }
}

fn change(author: Actor) -> ChangeRequest {
    ChangeRequest {
        item: item(),
        author,
        assigned_to_me: false,
        reviewers: Vec::new(),
        draft: false,
        threads: Vec::new(),
        my_last_activity_at: None,
        last_push_at: None,
        review_requested_at: None,
        approved: false,
        approvals_missing: 1,
        checks: Checks::Passed,
        conflicts: false,
        i_can_merge: true,
    }
}

fn mine() -> ChangeRequest {
    let mut change = change(Actor::me("me"));
    change.reviewers = vec![reviewer(Actor::other("till"), ReviewState::Pending)];
    change
}

fn theirs(state: ReviewState) -> ChangeRequest {
    let mut change = change(Actor::other("alex"));
    change.reviewers = vec![reviewer(Actor::me("me"), state)];
    change
}

fn reviewer(actor: Actor, state: ReviewState) -> Reviewer {
    Reviewer { actor, state }
}

fn note(author: Actor, at: i64) -> Note {
    Note {
        author,
        at: Timestamp(at),
        mentions_me: false,
    }
}

fn thread(notes: Vec<Note>) -> Thread {
    Thread {
        url: "https://example.test/mr/7#note_1".into(),
        resolvable: true,
        resolved: false,
        notes,
    }
}

fn run(change: ChangeRequest, rules: &Rules) -> Option<Task> {
    triage(&WorkItem::Change(change), rules, NOW)
}

fn kinds(task: &Option<Task>) -> Vec<Kind> {
    task.iter()
        .flat_map(|task| task.reasons.iter().map(|reason| reason.kind))
        .collect()
}

fn me() -> Actor {
    Actor::me("me")
}

fn alex() -> Actor {
    Actor::other("alex")
}

// ── as a reviewer ──────────────────────────────────────────────────────

#[test]
fn a_fresh_review_request_is_mine() {
    let task = run(theirs(ReviewState::Pending), &Rules::default());
    assert_eq!(kinds(&task), [Kind::ReviewRequested]);
    assert_eq!(task.unwrap().bucket(), Bucket::Todo);
}

#[test]
fn after_my_review_the_author_has_the_ball() {
    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![thread(vec![note(me(), 10)])];
    let task = run(change, &Rules::default());
    assert_eq!(kinds(&task), [Kind::AwaitingAuthor]);
    let task = task.unwrap();
    assert_eq!(task.bucket(), Bucket::Waiting);
    assert_eq!(task.headline().detail, Detail::Threads(1));
}

#[test]
fn an_answer_in_my_thread_asks_for_a_re_review_and_a_thread_to_close() {
    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![
        thread(vec![note(me(), 10), note(alex(), 20)]),
        thread(vec![note(me(), 11)]),
    ];
    let task = run(change, &Rules::default());
    assert_eq!(kinds(&task), [Kind::ReReview, Kind::ThreadToResolve]);
    let task = task.unwrap();
    assert_eq!(
        task.reasons[0].detail,
        Detail::Triggers(vec![Trigger::AuthorReplied])
    );
    assert_eq!(task.reasons[1].detail, Detail::Threads(1));
    assert_eq!(task.target_url(), "https://example.test/mr/7#note_1");
}

#[test]
fn commits_after_my_review_ask_for_a_re_review() {
    let mut change = theirs(ReviewState::Commented);
    change.my_last_activity_at = Some(Timestamp(10));
    change.last_push_at = Some(Timestamp(20));
    let task = run(change, &Rules::default());
    assert_eq!(
        task.unwrap().headline().detail,
        Detail::Triggers(vec![Trigger::NewCommits])
    );
}

#[test]
fn commits_before_my_review_do_not() {
    let mut change = theirs(ReviewState::Commented);
    change.my_last_activity_at = Some(Timestamp(30));
    change.last_push_at = Some(Timestamp(20));
    assert_eq!(
        kinds(&run(change, &Rules::default())),
        [Kind::AwaitingAuthor]
    );
}

#[test]
fn a_new_request_after_my_review_is_a_re_review_not_a_first_review() {
    let mut change = theirs(ReviewState::Pending);
    change.my_last_activity_at = Some(Timestamp(10));
    change.review_requested_at = Some(Timestamp(20));
    let task = run(change, &Rules::default());
    assert_eq!(
        task.unwrap().headline().detail,
        Detail::Triggers(vec![Trigger::ReRequested])
    );
}

#[test]
fn a_reset_approval_asks_for_a_re_review() {
    let mut change = theirs(ReviewState::ApprovalReset);
    change.my_last_activity_at = Some(Timestamp(10));
    let task = run(change, &Rules::default());
    assert_eq!(
        task.unwrap().headline().detail,
        Detail::Triggers(vec![Trigger::ApprovalReset])
    );
}

#[test]
fn only_the_configured_triggers_count() {
    let mut change = theirs(ReviewState::Commented);
    change.my_last_activity_at = Some(Timestamp(10));
    change.last_push_at = Some(Timestamp(20));
    let rules = Rules {
        re_review_triggers: vec![Trigger::ReRequested],
        ..Rules::default()
    };
    assert_eq!(kinds(&run(change, &rules)), [Kind::AwaitingAuthor]);
}

#[test]
fn all_threads_answered_waits_for_the_last_one() {
    let rules = Rules {
        re_review_triggers: vec![Trigger::AllThreadsAnswered],
        who_resolves: Resolver::Author,
        ..Rules::default()
    };
    let answered = thread(vec![note(me(), 10), note(alex(), 20)]);
    let pending = thread(vec![note(me(), 11)]);

    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![answered.clone(), pending.clone()];
    // One answer is in, but with the author resolving, it is only a reply to read.
    assert_eq!(kinds(&run(change, &rules)), [Kind::ReplyNeeded]);

    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![answered.clone(), answered];
    assert_eq!(
        kinds(&run(change, &rules)),
        [Kind::ReReview, Kind::ReplyNeeded]
    );
}

#[test]
fn approved_and_untouched_is_off_my_list() {
    let mut change = theirs(ReviewState::Approved);
    change.my_last_activity_at = Some(Timestamp(10));
    change.last_push_at = Some(Timestamp(20));
    assert_eq!(run(change, &Rules::default()), None);
}

#[test]
fn approved_can_be_re_reviewed_when_configured() {
    let mut change = theirs(ReviewState::Approved);
    change.my_last_activity_at = Some(Timestamp(10));
    change.last_push_at = Some(Timestamp(20));
    let rules = Rules {
        re_review_after_approval: true,
        ..Rules::default()
    };
    assert_eq!(kinds(&run(change, &rules)), [Kind::ReReview]);
}

#[test]
fn resolved_threads_ask_nothing() {
    let mut change = theirs(ReviewState::Commented);
    let mut closed = thread(vec![note(me(), 10), note(alex(), 20)]);
    closed.resolved = true;
    change.threads = vec![closed];
    assert_eq!(
        kinds(&run(change, &Rules::default())),
        [Kind::AwaitingAuthor]
    );
}

#[test]
fn bots_never_give_me_the_ball() {
    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![thread(vec![note(me(), 10), note(Actor::bot("ci"), 20)])];
    assert_eq!(
        kinds(&run(change.clone(), &Rules::default())),
        [Kind::AwaitingAuthor]
    );

    let rules = Rules {
        ignore_bots: false,
        ..Rules::default()
    };
    assert_eq!(
        kinds(&run(change, &rules)),
        [Kind::ReReview, Kind::ThreadToResolve]
    );
}

#[test]
fn someone_elses_draft_follows_the_draft_policy() {
    let mut change = theirs(ReviewState::Pending);
    change.draft = true;

    let wait = Rules::default();
    assert_eq!(kinds(&run(change.clone(), &wait)), [Kind::DraftByOther]);

    let ignore = Rules {
        others_drafts: OthersDrafts::Ignore,
        ..Rules::default()
    };
    assert_eq!(run(change.clone(), &ignore), None);

    let review = Rules {
        others_drafts: OthersDrafts::Review,
        ..Rules::default()
    };
    assert_eq!(kinds(&run(change, &review)), [Kind::ReviewRequested]);
}

#[test]
fn a_mention_in_a_thread_i_never_joined_asks_for_a_reply() {
    let mut change = theirs(ReviewState::Commented);
    change.my_last_activity_at = Some(Timestamp(10));
    let mut call = note(Actor::other("till"), 20);
    call.mentions_me = true;
    change.threads = vec![thread(vec![call])];
    assert_eq!(kinds(&run(change, &Rules::default())), [Kind::ReplyNeeded]);
}

// ── as the author ──────────────────────────────────────────────────────

#[test]
fn a_reviewer_comment_on_my_change_asks_for_a_reply() {
    let mut change = mine();
    change.threads = vec![thread(vec![note(Actor::other("till"), 10)])];
    let task = run(change, &Rules::default());
    assert_eq!(kinds(&task), [Kind::ReplyNeeded, Kind::AwaitingReview]);
    assert_eq!(task.unwrap().bucket(), Bucket::Todo);
}

#[test]
fn once_i_answered_the_reviewer_closes_or_i_do() {
    let mut change = mine();
    change.threads = vec![thread(vec![note(Actor::other("till"), 10), note(me(), 20)])];

    let by_reviewer = run(change.clone(), &Rules::default());
    assert_eq!(
        kinds(&by_reviewer),
        [Kind::AwaitingReview, Kind::AwaitingReply]
    );
    assert_eq!(by_reviewer.unwrap().bucket(), Bucket::Waiting);

    let rules = Rules {
        who_resolves: Resolver::Author,
        ..Rules::default()
    };
    let by_author = run(change, &rules);
    assert_eq!(
        kinds(&by_author),
        [Kind::ThreadToResolve, Kind::AwaitingReview]
    );
}

#[test]
fn waiting_for_review_names_who_has_not_looked() {
    let mut change = mine();
    change.approvals_missing = 2;
    change
        .reviewers
        .push(reviewer(Actor::other("carl"), ReviewState::Commented));
    let task = run(change, &Rules::default()).unwrap();
    assert_eq!(
        task.headline().detail,
        Detail::Review {
            missing: 2,
            pending: vec!["till".into()]
        }
    );
}

#[test]
fn red_checks_and_conflicts_are_mine() {
    let mut change = mine();
    change.checks = Checks::Failed;
    change.conflicts = true;
    assert_eq!(
        kinds(&run(change, &Rules::default())),
        [Kind::ChecksFailed, Kind::Conflicts, Kind::AwaitingReview]
    );
}

#[test]
fn approved_green_and_quiet_is_ready_to_merge() {
    let mut change = mine();
    change.approved = true;
    change.approvals_missing = 0;
    assert_eq!(
        kinds(&run(change.clone(), &Rules::default())),
        [Kind::ReadyToMerge]
    );

    change.threads = vec![thread(vec![note(Actor::other("till"), 10)])];
    assert_eq!(kinds(&run(change, &Rules::default())), [Kind::ReplyNeeded]);
}

#[test]
fn a_change_only_a_maintainer_can_merge_waits_on_them() {
    let mut change = mine();
    change.reviewers.clear();
    change.approved = true;
    change.approvals_missing = 0;
    change.i_can_merge = false;
    let task = run(change, &Rules::default()).unwrap();
    assert_eq!(task.bucket(), Bucket::Waiting);
    assert_eq!(
        task.headline().detail,
        Detail::Review {
            missing: 0,
            pending: vec![]
        }
    );
}

#[test]
fn a_bot_thread_i_answered_waits_on_nobody() {
    let mut change = mine();
    change.approved = true;
    change.approvals_missing = 0;
    change.threads = vec![thread(vec![note(Actor::bot("ci"), 10), note(me(), 20)])];
    // Nobody owes anybody an answer: the bot's thread does not hold the change.
    assert_eq!(
        kinds(&run(change.clone(), &Rules::default())),
        [Kind::ReadyToMerge]
    );

    // A human joining the bot's thread makes it a conversation again.
    change.threads[0].notes.push(note(Actor::other("till"), 30));
    assert_eq!(kinds(&run(change, &Rules::default())), [Kind::ReplyNeeded]);
}

#[test]
fn a_change_without_reviewer_needs_one() {
    let mut change = mine();
    change.reviewers.clear();
    assert_eq!(kinds(&run(change, &Rules::default())), [Kind::NoReviewer]);
}

#[test]
fn my_draft_is_mine_to_finish_and_nothing_else() {
    let mut change = mine();
    change.draft = true;
    assert_eq!(
        kinds(&run(change, &Rules::default())),
        [Kind::DraftToFinish]
    );
}

// ── rules ──────────────────────────────────────────────────────────────

#[test]
fn any_rule_can_be_disabled_moved_or_reprioritized() {
    let mut rules = Rules::default();
    rules.overrides.insert(
        Kind::DraftToFinish,
        RuleOverride {
            enabled: Some(false),
            ..RuleOverride::default()
        },
    );
    let mut draft = mine();
    draft.draft = true;
    assert_eq!(run(draft, &rules), None);

    rules.overrides.insert(
        Kind::ReviewRequested,
        RuleOverride {
            bucket: Some(Bucket::Waiting),
            priority: Some(1),
            ..RuleOverride::default()
        },
    );
    let task = run(theirs(ReviewState::Pending), &rules).unwrap();
    assert_eq!(task.bucket(), Bucket::Waiting);
    assert_eq!(task.priority(), 1);
}

#[test]
fn an_idle_waiting_item_becomes_a_nudge() {
    let mut change = mine();
    change.item.updated_at = Timestamp(90 * DAY);
    let rules = Rules {
        nudge_after_days: 7,
        ..Rules::default()
    };
    let task = run(change.clone(), &rules).unwrap();
    assert_eq!(task.bucket(), Bucket::Todo);
    assert_eq!(task.headline().kind, Kind::Nudge);
    assert_eq!(task.headline().detail, Detail::Idle(10));

    assert_eq!(
        kinds(&run(change, &Rules::default())),
        [Kind::AwaitingReview]
    );
}

// ── tickets ────────────────────────────────────────────────────────────

fn ticket(status: &str, category: StatusCategory) -> Ticket {
    Ticket {
        item: ItemRef {
            reference: "PROJ-120".into(),
            container: "PROJ".into(),
            source: "jira".into(),
            ..item()
        },
        status: status.into(),
        category,
        blocked_by: Vec::new(),
        mention_pending: false,
    }
}

fn run_ticket(ticket: Ticket) -> Option<Task> {
    triage(&WorkItem::Ticket(ticket), &Rules::default(), NOW)
}

#[test]
fn tickets_follow_their_status() {
    let todo = run_ticket(ticket("To Do", StatusCategory::Todo));
    assert_eq!(kinds(&todo), [Kind::TicketTodo]);
    let doing = run_ticket(ticket("In Progress", StatusCategory::InProgress));
    assert_eq!(kinds(&doing), [Kind::TicketInProgress]);
    assert_eq!(run_ticket(ticket("Done", StatusCategory::Done)), None);
}

#[test]
fn blocked_tickets_wait() {
    let by_status = run_ticket(ticket("In Review", StatusCategory::InProgress)).unwrap();
    assert_eq!(by_status.bucket(), Bucket::Waiting);

    let mut linked = ticket("To Do", StatusCategory::Todo);
    linked.blocked_by = vec!["PROJ-99".into()];
    let task = run_ticket(linked).unwrap();
    assert_eq!(
        task.headline().detail,
        Detail::BlockedBy(vec!["PROJ-99".into()])
    );
}

#[test]
fn a_mention_on_a_blocked_ticket_is_still_mine() {
    let mut blocked = ticket("Blocked", StatusCategory::InProgress);
    blocked.mention_pending = true;
    let task = run_ticket(blocked).unwrap();
    assert_eq!(task.bucket(), Bucket::Todo);
    assert_eq!(
        kinds(&Some(task)),
        [Kind::TicketMentioned, Kind::TicketBlocked]
    );
}
