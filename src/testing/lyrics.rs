//! Original lyrics written for the demo catalogue (the demo artists and songs
//! are fictional). Three are synced LRC, one is plain text.

pub fn for_track(id: &str) -> Option<&'static str> {
    match id {
        "1" => Some(NIGHT_DRIVE),
        "2" => Some(STATIC_BLOOM),
        "11" => Some(SEVERNOE_SIYANIE),
        "13" => Some(ELEKTRICHKA),
        _ => None,
    }
}

const STATIC_BLOOM: &str = "[ar:Neon Harbor]
[ti:Static Bloom]
[00:04.00]Static on the window, blooming like a rose
[00:11.00]every little signal finds a place to go
[00:18.00]I tune into the hum beneath the city lights
[00:25.00]and something in the noise tells me it's alright
[00:32.00]
[00:36.00]Bloom, bloom — the static in my head
[00:43.00]bloom, bloom — the words we never said
[00:50.00]turn it up until the silence fades
[00:57.00]we are the frequencies the night has made
[01:05.00]
[01:10.00]Copper wires humming in the rain
[01:17.00]tracing every line back home again
[01:24.00]if you can hear me, leave the dial alone
[01:31.00]I'm somewhere in between the tone and the drone
[01:38.00]
[01:42.00]Bloom, bloom — the static in my head
[01:49.00]bloom, bloom — the words we never said
[01:56.00]turn it up until the silence fades
[02:03.00]we are the frequencies the night has made
[02:11.00]
[02:20.00]And when the morning comes to wash it all away
[02:28.00]I'll keep a little crackle of the night we stayed
[02:36.00]static on the window
[02:42.00]blooming like a rose
[02:50.00]";

const NIGHT_DRIVE: &str = "[00:05.00]Headlights paint the overpass in gold
[00:12.00]radio is playing something old
[00:19.00]you put your hand out to the passing air
[00:26.00]like the road could take us anywhere
[00:34.00]
[00:38.00]Night drive, we don't need a map
[00:45.00]every exit's just a way back
[00:52.00]night drive, keep the engine low
[00:59.00]there's a city only we know
[01:07.00]
[01:12.00]Neon signs are spelling out our names
[01:19.00]flicker once and never look the same
[01:26.00]Night drive, we don't need a map
[01:33.00]every exit's just a way back
[01:41.00]";

const SEVERNOE_SIYANIE: &str = "[00:06.00]Над Ладогой тихо, вода как стекло
[00:13.00]и небо над нами зелёным свело
[00:20.00]ты смотришь наверх и не веришь глазам —
[00:27.00]там кто-то рисует по тёмным краям
[00:35.00]
[00:39.00]Северное сияние,
[00:44.00]тихое признание,
[00:49.00]светится над крышами,
[00:54.00]песня, что не слышали
[01:01.00]
[01:06.00]Мы ждали всю зиму короткую ночь,
[01:13.00]и ветер с залива не смог нам помочь,
[01:20.00]но лёд под ногами звенит, как струна —
[01:27.00]сегодня над нами горит тишина
[01:35.00]
[01:39.00]Северное сияние,
[01:44.00]тихое признание,
[01:49.00]светится над крышами,
[01:54.00]песня, что не слышали
[02:02.00]";

const ELEKTRICHKA: &str = "
Электричка уходит в шесть ноль пять,
в тамбуре холодно, нечего ждать.
За окном проплывают столбы и дома,
и платформы, где осень сошла с ума.

Мы едем туда, где кончается свет,
где на станциях нет ни людей, ни примет.
Перрон за перроном, гудок за гудком —
мы с тобою едем домой пешком.

Контролёр улыбнётся и пройдёт стороной,
у него тоже кто-то есть за спиной.
Электричка уходит, и ты не спеши —
у неё расписание для души.
";
