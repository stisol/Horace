use std::str::FromStr;
use std::{thread, time};

use serenity::model::channel::Message;
use serenity::model::id::UserId;
use chrono::Duration;
use chrono::offset::Utc;

use command_error::*;
use connectionpool::ConnectionPool;

static USAGE: &str = "Usage: `!remindme x scale`, where `x` is a number, \
                      and scale is `minutes`, `hours`, `days` or `weeks`.";

/// Stores a reminder in the database.
pub fn remindme(
    num: u32,
    scale: &str,
    message: &str,
    user_id: &UserId,
    pool: &mut ConnectionPool,
) -> Result<String, CommandError> {
    let user_id = format!("{}", user_id.0);

    let interval = match interval(num as i64, &scale) {
        Ok(i) => i,
        Err(why) => return Ok(why),
    };

    let date = Utc::now()
        .naive_utc()
        .checked_add_signed(interval)
        .ok_or(CommandError::Generic("Date overflow".to_owned()))?;

    // And ship it all to the database.
    let conn = pool.get_conn()?;
    conn.execute(
        "INSERT INTO reminders (user_id, date, message) VALUES ($1, $2, $3)",
        &[&user_id, &date, &message],
    )?;

    Ok(format!(
        "Reminder set for {} UTC.",
        date.format("%Y-%m-%d %H:%M:%S")
    ))
}

/// Attempts to parse the interval part from a `!remindme` command.
/// Currently only supports a `x minutes/hours/days/weeks` syntax.
fn interval(num: i64, scale: &str) -> Result<Duration, String> {
    match &*scale.to_lowercase() {
        "minutes" | "minute" => return Ok(Duration::minutes(num)),
        "hours" | "hour" => return Ok(Duration::hours(num)),
        "days" | "day" => return Ok(Duration::days(num)),
        "weeks" | "week" => return Ok(Duration::weeks(num)),
        _ => return Err(format!("Invalid duration scale.\n{}", USAGE)),
    }
}

/// Infinite loop that checks the database periodically for expired reminders.
pub fn watch_for_reminders(mut pool: ConnectionPool) -> ! {
    loop {
        thread::sleep(time::Duration::from_secs(60));

        // Get expired reminders.
        let rows = match get_expired_reminders(&mut pool) {
            Ok(rows) => rows,
            Err(why) => {
                error!("Failed to get reminders: {:?}", why);
                continue;
            }
        };

        // Send all reminders.
        for row in rows.into_iter() {
            if let Err(why) = dm_with_message(row.user_id, row.message) {
                error!("Error while DM'ing: {}", why);
            }
        }
    }
}

fn get_expired_reminders(pool: &mut ConnectionPool) -> Result<Vec<Reminder>, CommandError> {
    let conn = pool.get_conn()?;
    let rows = conn.query(
        "SELECT id, user_id, message FROM reminders WHERE date < current_timestamp",
        &[],
    )?;

    let rows: Vec<Reminder> = rows.into_iter()
        .map(|row| Reminder {
            id: row.get(0),
            user_id: row.get(1),
            message: row.get(2),
        })
        .collect();

    // Delete the reminder no matter if the reminder was sent sucessfully
    // or not to avoid retrying to message deleted accounts forever.
    for row in rows.iter() {
        conn.execute("DELETE FROM reminders WHERE id = $1", &[&row.id])?;
    }

    Ok(rows)
}

/// Parses a user_id and sends a reminder to the user.
fn dm_with_message(user_id: String, message: String) -> Result<(), String> {
    let userid = UserId::from_str(&user_id).map_err(|e| format!("Failed to get user id: {}", e))?;

    let user = userid
        .get()
        .map_err(|e| format!("Failed to get user: {}", e))?;

    let response = if message.is_empty() {
        "Hello! You asked me to remind you of something at this time,\n\
         but you didn't specify what!"
            .to_owned()
    } else {
        format!(
            "Hello! You asked me to remind you of the following: {}",
            message
        )
    };

    user.direct_message(|m| m.content(&response))
        .map_err(|why| format!("Failed to DM user: {}", why))?;

    Ok(())
}

struct Reminder {
    pub id: i32,
    pub user_id: String,
    pub message: String,
}
