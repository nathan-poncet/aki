use super::change_request::{ChangeRequest, Checks, Note, ReviewState, Reviewer, Thread};
use super::item::{Actor, ItemRef, WorkItem};
use super::rules::{OthersDrafts, OwnDrafts, Resolver, RuleOverride, Rules};
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
        created_at: Timestamp(50 * DAY),
        updated_at: Timestamp(99 * DAY),
    }
}

fn change(author: Actor) -> ChangeRequest {
    ChangeRequest {
        item: item(),
        branch: None,
        tickets: Vec::new(),
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
        behind_target: false,
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

#[test]
fn my_drafts_go_where_i_said() {
    let mut draft = mine();
    draft.draft = true;
    draft.behind_target = true;

    // With the rest, unless told otherwise.
    let task = run(draft.clone(), &Rules::default()).unwrap();
    assert!(task.draft && !task.apart);
    assert_eq!(kinds(&Some(task)), [Kind::Rebase, Kind::DraftToFinish]);

    // Apart: still everything there is to do on it, marked.
    let apart = Rules {
        own_drafts: OwnDrafts::Apart,
        ..Rules::default()
    };
    let task = run(draft.clone(), &apart).unwrap();
    assert!(task.draft && task.apart);
    assert_eq!(task.reasons.len(), 2);

    let ignore = Rules {
        own_drafts: OwnDrafts::Ignore,
        ..Rules::default()
    };
    assert_eq!(run(draft, &ignore), None);

    // Only my drafts: what is ready, and others' drafts, are untouched.
    let ready = run(mine(), &apart).unwrap();
    assert!(!ready.draft && !ready.apart);
    let mut theirs = theirs(ReviewState::Pending);
    theirs.draft = true;
    let task = run(theirs, &apart).unwrap();
    assert!(task.draft && !task.apart);
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
        priority: None,
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

// ── since when, and how urgent ─────────────────────────────────────────

#[test]
fn a_review_waits_since_it_was_asked_for() {
    let mut change = theirs(ReviewState::Pending);
    change.review_requested_at = Some(Timestamp(80 * DAY));
    let task = run(change, &Rules::default()).unwrap();
    assert_eq!(task.since, Timestamp(80 * DAY));
    // 20 days at 3 points, capped at 60, on top of the rule's 90.
    assert_eq!((task.urgency.rule, task.urgency.waiting), (90, 60));

    // Nobody recorded the request: since the change exists.
    let task = run(theirs(ReviewState::Pending), &Rules::default()).unwrap();
    assert_eq!(task.since, Timestamp(50 * DAY));
}

#[test]
fn a_re_review_waits_since_the_first_thing_that_made_it_due() {
    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![thread(vec![note(me(), 10), note(alex(), 97 * DAY)])];
    change.last_push_at = Some(Timestamp(98 * DAY));
    let task = run(change, &Rules::default()).unwrap();
    assert_eq!(task.headline().kind, Kind::ReReview);
    assert_eq!(task.since, Timestamp(97 * DAY));
    assert_eq!(task.urgency.waiting, 9);
}

#[test]
fn a_reply_waits_since_the_oldest_unanswered_word() {
    let mut change = mine();
    change.threads = vec![
        thread(vec![note(Actor::other("till"), 96 * DAY)]),
        thread(vec![note(Actor::other("till"), 90 * DAY)]),
    ];
    let task = run(change, &Rules::default()).unwrap();
    assert_eq!(task.headline().kind, Kind::ReplyNeeded);
    assert_eq!(task.since, Timestamp(90 * DAY));
}

#[test]
fn after_my_review_the_author_is_waited_on_since_my_last_word() {
    let mut change = theirs(ReviewState::Commented);
    change.threads = vec![thread(vec![note(me(), 95 * DAY)])];
    let task = run(change, &Rules::default()).unwrap();
    assert_eq!(task.since, Timestamp(95 * DAY));
}

#[test]
fn a_linked_ticket_and_its_priority_raise_a_change() {
    use super::ticket::{LinkedTicket, Priority};
    use super::urgency::Heat;

    let mut change = theirs(ReviewState::Pending);
    change.review_requested_at = Some(Timestamp(100 * DAY));
    let plain = run(change.clone(), &Rules::default()).unwrap();
    assert_eq!(
        (plain.urgency.score(), plain.urgency.heat),
        (90, Heat::Calm)
    );

    change.tickets = vec![LinkedTicket {
        key: "PROJ-7".into(),
        priority: Some(Priority::Highest),
    }];
    let urgent = run(change, &Rules::default()).unwrap();
    assert_eq!(
        (urgent.urgency.score(), urgent.urgency.heat),
        (145, Heat::Hot)
    );
    assert_eq!(urgent.tickets[0].key, "PROJ-7");
}

#[test]
fn a_ticket_weighs_its_own_priority() {
    use super::ticket::Priority;

    let mut high = ticket("To Do", StatusCategory::Todo);
    high.priority = Some(Priority::High);
    let task = run_ticket(high).unwrap();
    assert_eq!(task.priority_level, Some(Priority::High));
    // The rule's 20, a day of waiting, the priority's 30.
    assert_eq!(task.urgency.score(), 20 + 3 + 30);
}

#[test]
fn a_change_behind_its_target_is_mine_to_rebase() {
    let mut change = mine();
    change.behind_target = true;
    assert_eq!(
        kinds(&run(change.clone(), &Rules::default())),
        [Kind::Rebase, Kind::AwaitingReview]
    );

    // Conflicts say it already, and more.
    change.conflicts = true;
    assert_eq!(
        kinds(&run(change.clone(), &Rules::default())),
        [Kind::Conflicts, Kind::AwaitingReview]
    );

    // Approved and green, but not up to date: rebase before merging.
    change.conflicts = false;
    change.approved = true;
    change.approvals_missing = 0;
    assert_eq!(
        kinds(&run(change, &Rules::default())),
        [Kind::Rebase, Kind::ReadyToMerge]
    );

    // Someone else's branch is theirs to rebase.
    let mut theirs = theirs(ReviewState::Pending);
    theirs.behind_target = true;
    assert_eq!(
        kinds(&run(theirs, &Rules::default())),
        [Kind::ReviewRequested]
    );
}
